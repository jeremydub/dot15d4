#![allow(dead_code)]
use core::{marker::PhantomData, num::NonZero};

use dot15d4_driver::radio::DriverConfig;

#[cfg(feature = "rtos-trace")]
use crate::trace::MAC_REQUEST;
use crate::{
    driver::frame::{
        Address, AddressingMode, PanId, RadioFrame, RadioFrameSized, RadioFrameUnsized,
    },
    mac::{frame::mpdu::MpduFrame, MacService},
    scheduler::SchedulerServiceRequest,
    service::{ServiceTask, ServiceTaskEvent, ServiceTaskTransition},
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
    /// The frame recovered from Data Request.
    pub mpdu: MpduFrame,
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
    state: DataRequestState,
    task: PhantomData<&'task u8>,
    radio: PhantomData<RadioDriverImpl>,
}

pub(crate) enum DataRequestState {
    Initial(
        /// MPDU to be sent.
        MpduFrame,
    ),
    SendingRequest,
}

impl<'task, RadioDriverImpl: DriverConfig> DataRequestTask<'task, RadioDriverImpl> {
    pub fn new(data_request: DataRequest) -> Self {
        Self {
            state: DataRequestState::Initial(data_request.mpdu),
            task: PhantomData,
            radio: PhantomData,
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
    /// Not acknowledged: timeout or explicit NACK
    NoAck(
        /// recovered Tx radio frame
        RadioFrame<RadioFrameSized>,
    ),
}

/// Error that may occur during a Data Request task
pub(crate) enum DataRequestError {
    ChannelAccessFailure(
        /// unsent radio frame
        RadioFrame<RadioFrameSized>,
    ),
}

impl<'svc, RadioDriverImpl: DriverConfig + 'svc>
    ServiceTask<'svc, MacService<'svc, RadioDriverImpl>>
    for DataRequestTask<'svc, RadioDriverImpl>
{
    type Request = DataRequest;
    type Result = DataRequestResult;
    type Error = DataRequestError;

    fn step(
        mut self,
        event: ServiceTaskEvent<'svc, MacService<'svc, RadioDriverImpl>>,
    ) -> ServiceTaskTransition<'svc, MacService<'svc, RadioDriverImpl>, Self> {
        #[cfg(feature = "rtos-trace")]
        rtos_trace::trace::task_exec_begin(MAC_REQUEST);

        match self.state {
            DataRequestState::Initial(tx_mpdu) => {
                debug_assert!(matches!(event, ServiceTaskEvent::Entry));
                self.state = DataRequestState::SendingRequest;
                ServiceTaskTransition::Intermediate(
                    self,
                    SchedulerServiceRequest::Transmission(tx_mpdu),
                    None,
                )
            }
            DataRequestState::SendingRequest => match event {
                // ServiceTaskEvent::LowerServiceResponse(SchedSvcResponse::Transmission(result)) => {
                //     match result {
                //         Ok(task_result) => {
                //             let x = 1;
                //             match task_result {}
                //         }
                //         Err(error) => match error {},
                //     }
                // }
                _ => unreachable!(),
            },
        }
    }
}
