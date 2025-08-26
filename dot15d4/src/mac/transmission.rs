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

#[cfg(test)]
mod tests {
    use core::cell::RefCell;
    use dot15d4_frame::mpdu::MpduFrame;
    use dot15d4_util::allocator::IntoBuffer;

    use crate::mac::pib::Pib;
    use crate::mac::tests::{
        generate_data_frame, FakeDriverConfig, FakeRadioTimer, FakeRng, TaskTestEvent,
        TaskTestTransition, TaskTester,
    };
    use crate::mac::MacSvcContext;

    use super::{TransmissionResult, TransmissionTask};

    #[test]
    fn transmission_no_retransmission_success() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacSvcContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });
        let task = TransmissionTask::<FakeDriverConfig>::new(
            MpduFrame::from_radio_frame(radio_frame),
            &context,
        );

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxSent,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmissionResult::Sent(radio_frame) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => unreachable!("Unexpected Transmission task result"),
            }),
        );
    }

    #[test]
    fn transmission_no_retransmission_failure() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacSvcContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });
        let task = TransmissionTask::<FakeDriverConfig>::new(
            MpduFrame::from_radio_frame(radio_frame),
            &context,
        );

        context.borrow_mut().pib.max_csma_backoffs = 1;
        context.borrow_mut().pib.max_frame_retries = 0;

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmissionResult::ChannelAccessFailure(radio_frame) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => unreachable!("Unexpected Transmission task result"),
            }),
        );
    }

    #[test]
    fn transmission_no_retransmission_noack() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacSvcContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });
        let task = TransmissionTask::<FakeDriverConfig>::new(
            MpduFrame::from_radio_frame(radio_frame),
            &context,
        );

        context.borrow_mut().pib.max_csma_backoffs = 1;
        context.borrow_mut().pib.max_frame_retries = 0;

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmissionResult::NoAck(radio_frame) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => unreachable!("Unexpected Transmission task result"),
            }),
        );
    }

    #[test]
    fn transmission_one_retransmission_success() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacSvcContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });

        context.borrow_mut().pib.max_csma_backoffs = 1;
        context.borrow_mut().pib.max_frame_retries = 1;

        let task = TransmissionTask::<FakeDriverConfig>::new(
            MpduFrame::from_radio_frame(radio_frame),
            &context,
        );

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::DrvReqTx(&|task_tx, result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                match result.unwrap() {
                    TransmissionResult::Retransmitting(attempt) => {
                        assert_eq!(attempt, 2);
                    }
                    _ => unreachable!("Expected Retransmitting state in Transmission Task"),
                };
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxSent,
            TaskTestTransition::TaskTerminated(&|result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                match result {
                    TransmissionResult::Sent(radio_frame) => {
                        unsafe { radio_frame.into_buffer().consume() };
                    }
                    _ => unreachable!(),
                }
            }),
        );
    }

    #[test]
    fn transmission_one_retransmission_failure() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacSvcContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });
        let task = TransmissionTask::<FakeDriverConfig>::new(
            MpduFrame::from_radio_frame(radio_frame),
            &context,
        );

        context.borrow_mut().pib.max_csma_backoffs = 1;
        context.borrow_mut().pib.max_frame_retries = 1;

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::DrvReqTx(&|task_tx, result| {
                // TODO check timestamps
                match result.unwrap() {
                    TransmissionResult::Retransmitting(attempt) => {
                        assert_eq!(attempt, 2);
                    }
                    _ => unreachable!("Expected Retransmitting state in Transmission Task"),
                };
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmissionResult::ChannelAccessFailure(radio_frame) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => unreachable!("Unexpected Transmission task result"),
            }),
        );
    }

    #[test]
    fn transmission_one_retransmission_noack() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacSvcContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });
        let task = TransmissionTask::<FakeDriverConfig>::new(
            MpduFrame::from_radio_frame(radio_frame),
            &context,
        );

        context.borrow_mut().pib.max_csma_backoffs = 1;
        context.borrow_mut().pib.max_frame_retries = 1;

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::DrvReqTx(&|task_tx, result| {
                // TODO check timestamps
                match result.unwrap() {
                    TransmissionResult::Retransmitting(attempt) => {
                        assert_eq!(attempt, 2);
                    }
                    _ => unreachable!("Expected Retransmitting state in Transmission Task"),
                };
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmissionResult::NoAck(radio_frame) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => unreachable!("Unexpected Transmission task result"),
            }),
        );
    }

    #[test]
    fn transmission_multiple_retransmission_success() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacSvcContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });
        let task = TransmissionTask::<FakeDriverConfig>::new(
            MpduFrame::from_radio_frame(radio_frame),
            &context,
        );

        context.borrow_mut().pib.max_csma_backoffs = 1;
        context.borrow_mut().pib.max_frame_retries = 2;

        let mut tester = TaskTester::new(task);

        // First attempt
        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        // Second attempt, first retransmission
        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::DrvReqTx(&|task_tx, result| {
                // TODO check timestamps
                match result.unwrap() {
                    TransmissionResult::Retransmitting(attempt) => {
                        assert_eq!(attempt, 2);
                    }
                    _ => unreachable!("Expected Retransmitting state in Transmission Task"),
                };
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        // Third attempt, second retransmission
        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::DrvReqTx(&|task_tx, result| {
                // TODO check timestamps
                match result.unwrap() {
                    TransmissionResult::Retransmitting(attempt) => {
                        assert_eq!(attempt, 3);
                    }
                    _ => unreachable!("Expected Retransmitting state in Transmission Task"),
                };
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxSent,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmissionResult::Sent(radio_frame) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => unreachable!("Unexpected Transmission task result"),
            }),
        );
    }

    #[test]
    fn transmission_multiple_retransmission_failure() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacSvcContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });
        let task = TransmissionTask::<FakeDriverConfig>::new(
            MpduFrame::from_radio_frame(radio_frame),
            &context,
        );

        context.borrow_mut().pib.max_csma_backoffs = 1;
        context.borrow_mut().pib.max_frame_retries = 2;

        let mut tester = TaskTester::new(task);

        // First attempt
        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                // assert_eq!(task_tx.at, Timestamp::Scheduled(Instant::new(0)));
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        // Second attempt, first retransmission
        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::DrvReqTx(&|task_tx, result| {
                // TODO check timestamps
                match result.unwrap() {
                    TransmissionResult::Retransmitting(attempt) => {
                        assert_eq!(attempt, 2);
                    }
                    _ => unreachable!("Expected Retransmitting state in Transmission Task"),
                };
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        // Third attempt, second retransmission
        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::DrvReqTx(&|task_tx, result| {
                // TODO check timestamps
                match result.unwrap() {
                    TransmissionResult::Retransmitting(attempt) => {
                        assert_eq!(attempt, 3);
                    }
                    _ => unreachable!("Expected Retransmitting state in Transmission Task"),
                };
                task_tx
            }),
        );
        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                // TODO check timestamps
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmissionResult::ChannelAccessFailure(radio_frame) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => unreachable!("Unexpected Transmission task result"),
            }),
        );
    }
}
