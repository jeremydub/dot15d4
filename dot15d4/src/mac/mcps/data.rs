#![allow(dead_code)]
use core::{cell::RefCell, marker::PhantomData, num::NonZero};

#[cfg(feature = "rtos-trace")]
use crate::trace::{
    MAC_INDICATION, MAC_REQUEST, RX_CRC_ERROR, RX_FRAME, RX_INVALID, RX_WINDOW_ENDED,
};
use crate::{
    driver::{
        frame::{
            Address, AddressingMode, PanId, RadioFrame, RadioFrameRepr, RadioFrameSized,
            RadioFrameUnsized,
        },
        radio::DriverConfig,
        tasks::{RxError, RxResult, Timestamp},
        DrvSvcRequest, DrvSvcResponse, DrvSvcTaskError, DrvSvcTaskRx,
    },
    mac::{
        frame::mpdu::MpduFrame,
        task::*,
        transmission::{TransmissionResult, TransmissionTask},
        MacBufferAllocator, MacSvcContext,
    },
    util::{Error, Result as SimplifiedResult},
};

pub struct DataRequest {
    /// The frame to be sent.
    mpdu: MpduFrame,
}

/// Represents an MLME-DATA.request.
///
/// Note: Parameters that determine frame structure are currently read-only. We
///       may introduce structural writers if required. These should then safely
///       move existing data around.
impl DataRequest {
    pub fn new(mpdu: MpduFrame) -> Self {
        Self { mpdu }
    }

    pub fn src_addr_mode(&self) -> AddressingMode {
        self.mpdu.frame_control().src_addressing_mode()
    }

    pub fn dst_addr_mode(&self) -> AddressingMode {
        self.mpdu.frame_control().dst_addressing_mode()
    }

    pub fn dst_pan_id(&self) -> SimplifiedResult<PanId<&[u8]>> {
        self.mpdu
            .reader()
            .parse_addressing()?
            .try_into_addressing_fields()?
            .try_into_dst_pan_id()
            .ok_or(Error)
    }

    pub fn set_dst_pan_id<Bytes: AsRef<[u8]>>(
        &mut self,
        pan_id: PanId<Bytes>,
    ) -> SimplifiedResult<()> {
        self.mpdu
            .writer()
            .parse_addressing_mut()?
            .try_addressing_fields_mut()?
            .try_dst_pan_id_mut()
            .ok_or(Error)?
            .set_le_bytes(pan_id.as_ref());
        Ok(())
    }

    pub fn dst_addr(&self) -> SimplifiedResult<Address<&[u8]>> {
        self.mpdu
            .reader()
            .parse_addressing()?
            .try_into_addressing_fields()?
            .try_into_dst_address()
            .ok_or(Error)
    }

    pub fn set_dst_addr<Bytes: AsRef<[u8]>>(
        &mut self,
        dst_addr: &Address<Bytes>,
    ) -> SimplifiedResult<()> {
        let mut writer = self.mpdu.writer().parse_addressing_mut()?;
        let mut addr_fields = writer.try_addressing_fields_mut()?;
        addr_fields
            .try_dst_address_mut()
            .ok_or(Error)?
            .try_set(dst_addr)
    }

    pub fn tx_options(&mut self) -> TxOptions<'_> {
        TxOptions {
            mpdu: &mut self.mpdu,
        }
    }
}

pub struct TxOptions<'mpdu> {
    mpdu: &'mpdu mut MpduFrame,
}

impl<'mpdu> TxOptions<'mpdu> {
    pub fn ack_tx(&self) -> bool {
        self.mpdu.frame_control().ack_request()
    }

    pub fn set_ack_tx(&mut self, ack_tx: bool) {
        self.mpdu.frame_control_mut().set_ack_request(ack_tx);
    }

    pub fn pan_id_suppressed(&self) -> bool {
        self.mpdu.frame_control().pan_id_compression()
    }

    pub fn set_pan_id_suppressed(&mut self, pan_id_suppressed: bool) {
        self.mpdu
            .frame_control_mut()
            .set_pan_id_compression(pan_id_suppressed);
    }

    pub fn seq_num_suppressed(&self) -> bool {
        self.mpdu.frame_control().sequence_number_suppression()
    }

    pub fn set_seq_num_suppressed(&mut self, seq_num_suppressed: bool) {
        self.mpdu
            .frame_control_mut()
            .set_sequence_number_suppression(seq_num_suppressed);
    }
}

pub struct DataConfirm {
    /// Timestamp of frame transmission
    pub timestamp: Option<NonZero<u32>>,
    /// Whether the frame has been acknowledged or not
    pub acked: bool,
}

pub struct DataIndication {
    /// The received frame.
    pub mpdu: MpduFrame,
    /// Timestamp of frame reception
    pub timestamp: Option<NonZero<u32>>,
}

pub(crate) struct DataRequestTask<'task, RadioDriverImpl: DriverConfig> {
    state: DataRequestState<'task, RadioDriverImpl>,
}

pub(crate) enum DataRequestState<'task, RadioDriverImpl: DriverConfig> {
    Initial(
        /// MPDU to be sent.
        MpduFrame,
        /// MAC Service context
        &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
        /// Placeholder for future references.
        PhantomData<&'task RadioDriverImpl>,
    ),
    SendingRequest(TransmissionTask<'task, RadioDriverImpl>),
}

impl<'task, RadioDriverImpl: DriverConfig> DataRequestTask<'task, RadioDriverImpl> {
    pub fn new(
        data_request: DataRequest,
        context: &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
    ) -> Self {
        Self {
            state: DataRequestState::Initial(data_request.mpdu, context, PhantomData),
        }
    }
}

/// Final result of a data request task.
pub(crate) enum DataRequestResult {
    /// The Tx frame was sent.
    ///
    /// If ACK was requested, this result will only be returned if the frame was
    /// successfully acknowledged. Otherwise this result merely indicates that
    /// the frame was accepted by the driver and transmitted over the air.
    Sent(
        /// recovered Tx radio frame
        RadioFrame<RadioFrameUnsized>,
    ),
    ChannelAccessFailure(
        /// unsent radio frame
        RadioFrame<RadioFrameSized>,
    ),
    /// Not acknowledged: timeout or explicit NACK
    NoAck(
        /// recovered Tx radio frame
        RadioFrame<RadioFrameSized>,
    ),
}

impl<'task, RadioDriverImpl: DriverConfig> MacTask for DataRequestTask<'task, RadioDriverImpl> {
    type Result = DataRequestResult;

    fn step(mut self, event: MacTaskEvent) -> MacTaskTransition<Self> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(MAC_REQUEST);

        match self.state {
            DataRequestState::Initial(tx_mpdu, context, _) => {
                debug_assert!(matches!(event, MacTaskEvent::Entry));
                match TransmissionTask::<RadioDriverImpl>::new(tx_mpdu, context)
                    .step(MacTaskEvent::Entry)
                {
                    MacTaskTransition::DrvSvcRequest(transmission_task, drv_svc_request, _) => {
                        self.state = DataRequestState::SendingRequest(transmission_task);
                        MacTaskTransition::DrvSvcRequest(self, drv_svc_request, None)
                    }
                    MacTaskTransition::Terminated(_) => unreachable!(),
                }
            }
            DataRequestState::SendingRequest(transmission_task) => {
                match transmission_task.step(event) {
                    MacTaskTransition::Terminated(transmission_result) => match transmission_result
                    {
                        TransmissionResult::Sent(radio_frame) => {
                            MacTaskTransition::Terminated(DataRequestResult::Sent(radio_frame))
                        }
                        TransmissionResult::ChannelAccessFailure(radio_frame) => {
                            MacTaskTransition::Terminated(DataRequestResult::ChannelAccessFailure(
                                radio_frame,
                            ))
                        }
                        TransmissionResult::NoAck(radio_frame) => {
                            MacTaskTransition::Terminated(DataRequestResult::NoAck(radio_frame))
                        }
                        _ => unreachable!(),
                    },
                    MacTaskTransition::DrvSvcRequest(transmission_task, drv_svc_request, _) => {
                        self.state = DataRequestState::SendingRequest(transmission_task);
                        MacTaskTransition::DrvSvcRequest(self, drv_svc_request, None)
                    }
                }
            }
        }
    }
}

pub(crate) struct DataIndicationTask<'task, RadioDriverImpl: DriverConfig> {
    buffer_allocator: MacBufferAllocator,
    state: DataIndicationState<'task, RadioDriverImpl>,
}

enum DataIndicationState<'task, RadioDriverImpl: DriverConfig> {
    // Placeholder for future references.
    Initial(PhantomData<&'task RadioDriverImpl>),
    WaitingForFrame,
}

impl<'task, RadioDriverImpl: DriverConfig> DataIndicationTask<'task, RadioDriverImpl> {
    pub fn new(buffer_allocator: MacBufferAllocator) -> Self {
        Self {
            buffer_allocator,
            state: DataIndicationState::Initial(PhantomData),
        }
    }

    fn allocate_rx_radio_frame(
        buffer_allocator: &MacBufferAllocator,
    ) -> Option<RadioFrame<RadioFrameUnsized>> {
        let rx_buffer = buffer_allocator.try_allocate_buffer(
            RadioFrameRepr::<RadioDriverImpl, RadioFrameUnsized>::new().max_buffer_length()
                as usize,
        );

        if let Ok(rx_buffer) = rx_buffer {
            Some(RadioFrame::<RadioFrameUnsized>::new::<RadioDriverImpl>(
                rx_buffer,
            ))
        } else {
            None
        }
    }

    fn handle_rx_driver_response(
        &self,
        response: DrvSvcResponse,
    ) -> Result<MpduFrame, RadioFrame<RadioFrameUnsized>> {
        match response {
            DrvSvcResponse::Rx(rx_result) => match rx_result {
                Ok(rx_result) => match rx_result {
                    RxResult::Frame(rx_frame) => {
                        #[cfg(feature = "rtos-trace")]
                        rtos_trace::trace::marker(RX_FRAME);

                        let mpdu = MpduFrame::from_radio_frame(rx_frame);
                        Ok(mpdu)
                    }
                    RxResult::FilteredFrame(recovered_radio_frame) => {
                        #[cfg(feature = "rtos-trace")]
                        rtos_trace::trace::marker(RX_INVALID);

                        Err(recovered_radio_frame.forget_size::<RadioDriverImpl>())
                    }
                    RxResult::RxWindowEnded(recovered_radio_frame) => {
                        #[cfg(feature = "rtos-trace")]
                        rtos_trace::trace::marker(RX_WINDOW_ENDED);

                        Err(recovered_radio_frame)
                    }
                    RxResult::CrcError(recovered_radio_frame) => {
                        #[cfg(feature = "rtos-trace")]
                        rtos_trace::trace::marker(RX_CRC_ERROR);

                        Err(recovered_radio_frame)
                    }
                },
                Err(rx_task_error) => match rx_task_error {
                    // Bailing CRC errors should be handled by the driver
                    // service.
                    DrvSvcTaskError::Task(RxError::CrcError) => unreachable!(),
                    // TODO: Implement if required by a driver implementation.
                    _ => unreachable!(),
                },
            },
            // Safety: We scheduled an Rx task and therefore expect an Rx task
            //         response.
            _ => unreachable!(),
        }
    }

    fn produce_indication_and_restart_rx(
        rx_mpdu: MpduFrame,
        buffer_allocator: MacBufferAllocator,
    ) -> MacTaskTransition<Self> {
        let data_indication = DataIndication {
            mpdu: rx_mpdu,
            timestamp: None,
        };
        let next_rx_radio_frame =
            Self::allocate_rx_radio_frame(&buffer_allocator).expect("no capacity");
        MacTaskTransition::DrvSvcRequest(
            Self {
                buffer_allocator,
                state: DataIndicationState::WaitingForFrame,
            },
            Self::rx_task(next_rx_radio_frame),
            Some(data_indication),
        )
    }

    fn rx_task(radio_frame: RadioFrame<RadioFrameUnsized>) -> DrvSvcRequest {
        DrvSvcTaskRx {
            start: Timestamp::BestEffort,
            radio_frame,
        }
        .into()
    }
}

impl<RadioDriverImpl: DriverConfig> MacTask for DataIndicationTask<'_, RadioDriverImpl> {
    type Result = DataIndication;

    fn step(mut self, event: MacTaskEvent) -> MacTaskTransition<Self> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(MAC_INDICATION);

        match self.state {
            DataIndicationState::Initial(_) => {
                debug_assert!(matches!(event, MacTaskEvent::Entry));

                let rx_radio_frame =
                    Self::allocate_rx_radio_frame(&self.buffer_allocator).expect("no capacity");
                self.state = DataIndicationState::WaitingForFrame;
                MacTaskTransition::DrvSvcRequest(self, Self::rx_task(rx_radio_frame), None)
            }
            DataIndicationState::WaitingForFrame => match event {
                MacTaskEvent::DrvSvcResponse(driver_response) => {
                    match self.handle_rx_driver_response(driver_response) {
                        // We successfully received an MPDU.
                        Ok(rx_mpdu) => {
                            self.state = DataIndicationState::WaitingForFrame;
                            Self::produce_indication_and_restart_rx(rx_mpdu, self.buffer_allocator)
                        }
                        // The previous Rx task ended without receiving a valid
                        // frame. Start waiting for the next frame.
                        Err(recovered_rx_radio_frame) => {
                            // Wait for the next frame
                            self.state = DataIndicationState::WaitingForFrame;
                            MacTaskTransition::DrvSvcRequest(
                                self,
                                Self::rx_task(recovered_rx_radio_frame),
                                None,
                            )
                        }
                    }
                }
                // Safety: We issued an Rx task and therefore expect an Rx result.
                _ => unreachable!(),
            },
        }
    }
}
