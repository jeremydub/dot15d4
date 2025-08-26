#![allow(dead_code)]
use core::{cell::RefCell, marker::PhantomData};

use dot15d4_driver::radio::DriverConfig;

use crate::{
    driver::frame::{RadioFrame, RadioFrameSized, RadioFrameUnsized},
    mac::{frame::mpdu::MpduFrame, task::*},
};

use super::{
    csma::{TransmitWithCsmaCaResult, TransmitWithCsmaCaTask},
    MacSvcContext,
};

pub(crate) struct TransmissionTask<'task, RadioDriverImpl: DriverConfig> {
    state: TransmissionState<'task, RadioDriverImpl>,
    radio: PhantomData<RadioDriverImpl>,
}

pub(crate) enum TransmissionState<'task, RadioDriverImpl: DriverConfig> {
    Initial(
        /// MPDU to be sent.
        MpduFrame,
        /// MAC Service context
        &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
        /// Placeholder for future references.
        PhantomData<&'task RadioDriverImpl>,
    ),
    SendingFrameWithCsmaCa(
        u8,
        TransmitWithCsmaCaTask<'task, RadioDriverImpl>,
        &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
    ),
}

impl<'task, RadioDriverImpl: DriverConfig> TransmissionTask<'task, RadioDriverImpl> {
    pub fn new(
        mpdu: MpduFrame,
        context: &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
    ) -> Self {
        Self {
            state: TransmissionState::Initial(mpdu, context, PhantomData),
            radio: PhantomData,
        }
    }
}

/// Final result of a data request task.
#[derive(Debug, PartialEq)]
pub(crate) enum TransmissionResult {
    /// The Tx frame was sent.
    ///
    /// If ACK was requested, this result will only be returned if the frame was
    /// successfully acknowledged. Otherwise this result merely indicates that
    /// the frame was accepted by the driver and transmitted over the air.
    Sent(
        /// recovered Tx radio frame
        RadioFrame<RadioFrameUnsized>,
    ),
    /// Retransmitting frame after a first failed attempt (using CSMA-CA)
    ///
    /// This is always an intermediate result
    Retransmitting(
        /// retransmission attempt
        u8,
    ),
    /// Not acknowledged: timeout or explicit NACK
    NoAck(
        /// recovered Tx radio frame
        RadioFrame<RadioFrameSized>,
    ),
    /// Failed after maximum number of attempts.
    ///
    /// This is always a final result.
    ChannelAccessFailure(RadioFrame<RadioFrameSized>),
}

impl<'task, RadioDriverImpl: DriverConfig> MacTask for TransmissionTask<'task, RadioDriverImpl> {
    type Result = TransmissionResult;

    fn step(mut self, event: MacTaskEvent) -> MacTaskTransition<Self> {
        match self.state {
            TransmissionState::Initial(mpdu, context, _) => {
                debug_assert!(matches!(event, MacTaskEvent::Entry));
                let radio_frame = mpdu.into_radio_frame::<RadioDriverImpl>();
                match TransmitWithCsmaCaTask::<RadioDriverImpl>::new(radio_frame, context)
                    .step(MacTaskEvent::Entry)
                {
                    MacTaskTransition::DrvSvcRequest(csma_ca_task, drv_svc_request, _) => {
                        self.state =
                            TransmissionState::SendingFrameWithCsmaCa(1, csma_ca_task, context);
                        MacTaskTransition::DrvSvcRequest(self, drv_svc_request, None)
                    }
                    MacTaskTransition::Terminated(_) => unreachable!(),
                }
            }
            TransmissionState::SendingFrameWithCsmaCa(attempt, csma_ca_task, context) => {
                match csma_ca_task.step(event) {
                    MacTaskTransition::DrvSvcRequest(
                        csma_ca_task,
                        drv_svc_request,
                        Some(TransmitWithCsmaCaResult::CsmaCaBackoff(_nb)),
                    ) => {
                        self.state = TransmissionState::SendingFrameWithCsmaCa(
                            attempt,
                            csma_ca_task,
                            context,
                        );
                        MacTaskTransition::DrvSvcRequest(self, drv_svc_request, None)
                    }
                    MacTaskTransition::Terminated(csma_ca_result) => match csma_ca_result {
                        TransmitWithCsmaCaResult::Sent(radio_frame, _nb) => {
                            // TODO: propagate NB
                            MacTaskTransition::Terminated(TransmissionResult::Sent(
                                radio_frame.forget_size::<RadioDriverImpl>(),
                            ))
                        }
                        TransmitWithCsmaCaResult::ChannelAccessFailure(radio_frame) => {
                            MacTaskTransition::Terminated(TransmissionResult::ChannelAccessFailure(
                                radio_frame,
                            ))
                        }
                        TransmitWithCsmaCaResult::NoAck(radio_frame) => {
                            if attempt <= context.borrow().pib.max_frame_retries {
                                // TODO: support non-CsmaCa based transmisson
                                match TransmitWithCsmaCaTask::<RadioDriverImpl>::new(
                                    radio_frame,
                                    context,
                                )
                                .step(MacTaskEvent::Entry)
                                {
                                    MacTaskTransition::DrvSvcRequest(
                                        csma_ca_task,
                                        drv_svc_request,
                                        _,
                                    ) => {
                                        self.state = TransmissionState::SendingFrameWithCsmaCa(
                                            attempt + 1,
                                            csma_ca_task,
                                            context,
                                        );
                                        MacTaskTransition::DrvSvcRequest(
                                            self,
                                            drv_svc_request,
                                            Some(TransmissionResult::Retransmitting(attempt + 1)),
                                        )
                                    }
                                    MacTaskTransition::Terminated(_) => unreachable!(),
                                }
                            } else {
                                MacTaskTransition::Terminated(TransmissionResult::NoAck(
                                    radio_frame,
                                ))
                            }
                        }
                        // Backoff Result is not a final result
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                }
            }
        }
    }
}
