#![allow(dead_code)]
use core::{cell::RefCell, cmp::min, marker::PhantomData};

use dot15d4_driver::{
    constants::{MAC_UNIT_BACKOFF_PERIOD, PHY_CCA_DURATION},
    frame::{RadioFrame, RadioFrameSized},
    radio::DriverConfig,
    tasks::TaskTx,
    timer::{LocalClockDuration, LocalClockInstant, RadioTimerApi},
};
use dot15d4_frame::mpdu::MpduFrame;

use crate::{
    driver::{
        tasks::{Timestamp, TxError, TxResult},
        DrvSvcRequest, DrvSvcResponse, DrvSvcTaskError, DrvSvcTaskTx,
    },
    service::{ServiceTask, ServiceTaskEvent, ServiceTaskTransition},
    MacContext,
};

#[cfg(feature = "rtos-trace")]
use crate::trace::{TX_CCABUSY, TX_FRAME, TX_NACK};

use super::SchedulerService;

// TODO: make it configurable
// Maximum time it takes to communicate the task from the MAC service to the
// driver in debug mode.
const SERVICE_GUARD_TIME: LocalClockDuration = LocalClockDuration::micros(500);

#[derive(Debug, PartialEq)]
pub(crate) enum TransmitWithCsmaCaResult {
    /// The Tx frame was sent and acknowledged.
    ///
    /// This is always a final result.
    Sent(
        /// recovered Tx radio frame
        RadioFrame<RadioFrameSized>,
        /// backoffs
        u8,
    ),
    /// The Tx frame was sent but not acknowledged.
    ///
    /// This is always a final result.
    NoAck(
        /// recovered Tx radio frame
        RadioFrame<RadioFrameSized>,
    ),
    /// Transmission attempt failed.
    ///
    /// This is always an intermediate result.
    CsmaCaBackoff(
        /// Attempt number
        u8,
    ),
}

#[derive(Debug, PartialEq)]
pub(crate) enum TransmitWithCsmaCaError {
    /// Failed after maximum number of attempts.
    ///
    /// This is always a final result.
    ChannelAccessFailure(RadioFrame<RadioFrameSized>),
}

pub(crate) enum TransmitWithCsmaCaState<'task, RadioDriverImpl: DriverConfig> {
    Initial(
        /// Radio frame to be sent.
        RadioFrame<RadioFrameSized>,
        /// MAC Service context
        &'task RefCell<MacContext<'task, RadioDriverImpl>>,
    ),
    AttemptingTransmission(
        u8,
        u8,
        LocalClockInstant,
        &'task RefCell<MacContext<'task, RadioDriverImpl>>,
    ),
}

pub(crate) struct TransmitWithCsmaCaTask<'task, RadioDriverImpl: DriverConfig> {
    state: TransmitWithCsmaCaState<'task, RadioDriverImpl>,
    radio: PhantomData<RadioDriverImpl>,
}

impl<'svc, RadioDriverImpl: DriverConfig> TransmitWithCsmaCaTask<'svc, RadioDriverImpl> {
    pub(super) fn new(
        radio_frame: RadioFrame<RadioFrameSized>,
        context: &'svc RefCell<MacContext<'svc, RadioDriverImpl>>,
    ) -> Self {
        Self {
            state: TransmitWithCsmaCaState::Initial(radio_frame, context),
            radio: PhantomData,
        }
    }

    fn next_backoff_duration(
        context: &'svc RefCell<MacContext<'svc, RadioDriverImpl>>,
        backoff_exponent: u8,
    ) -> LocalClockDuration {
        let mut context = context.borrow_mut();
        // Number of backoff periods between 0 and (2^BE)-1
        let n_backoff = context.rng.next_u32() % 2u32.pow(backoff_exponent.into());

        LocalClockDuration::from_ticks(0)
            .checked_add(MAC_UNIT_BACKOFF_PERIOD * n_backoff)
            .unwrap()
            .checked_add(PHY_CCA_DURATION)
            .unwrap()
    }

    /// Represents RMARKER time of the first transmission, without any backoff.
    /// Takes into consideration driver/CPU guard time.
    fn anchor_time(context: &'svc RefCell<MacContext<'svc, RadioDriverImpl>>) -> LocalClockInstant {
        context
            .borrow()
            .timer
            .now()
            .checked_add_duration(SERVICE_GUARD_TIME)
            .unwrap()
            .checked_add_duration(RadioDriverImpl::GUARD_TIME)
            .unwrap()
    }

    fn handle_tx_error(
        mut self,
        tx_error: DrvSvcTaskError<DrvSvcTaskTx>,
        nb: u8,
        be: u8,
        anchor_time: LocalClockInstant,
        context: &'svc RefCell<MacContext<'svc, RadioDriverImpl>>,
    ) -> ServiceTaskTransition<SchedulerService<RadioDriverImpl>, Self> {
        let mut next_be = min(be + 1, context.borrow().pib.max_be);
        let mut nb = nb;
        // Anchor time to use for next attempt depending on the TX error, and recovered radio frame
        let (next_anchor_time, radio_frame) = match tx_error {
            DrvSvcTaskError::Task(TxError::CcaBusy(radio_frame)) => {
                #[cfg(feature = "rtos-trace")]
                rtos_trace::trace::marker(TX_CCABUSY);
                nb += 1;
                (
                    anchor_time
                        .checked_add_duration(TransmitWithCsmaCaTask::next_backoff_duration(
                            context, next_be,
                        ))
                        .unwrap(),
                    radio_frame,
                )
            }
            DrvSvcTaskError::SchedulingError(task) => {
                if nb == 0 {
                    next_be = context.borrow().pib.min_be;
                    (
                        TransmitWithCsmaCaTask::anchor_time(context)
                            .checked_add_duration(TransmitWithCsmaCaTask::next_backoff_duration(
                                context, next_be,
                            ))
                            .unwrap(),
                        task.radio_frame,
                    )
                } else {
                    nb += 1;
                    (
                        anchor_time
                            .checked_add_duration(TransmitWithCsmaCaTask::next_backoff_duration(
                                context, next_be,
                            ))
                            .unwrap(),
                        task.radio_frame,
                    )
                }
            }
        };
        if nb > context.borrow().pib.max_csma_backoffs {
            return ServiceTaskTransition::Terminated(Err(
                TransmitWithCsmaCaError::ChannelAccessFailure(radio_frame),
            ));
        }

        self.state =
            TransmitWithCsmaCaState::AttemptingTransmission(nb, next_be, next_anchor_time, context);
        ServiceTaskTransition::Intermediate(
            self,
            DrvSvcRequest::Tx(TaskTx {
                at: Timestamp::Scheduled(next_anchor_time),
                radio_frame,
                cca: true,
            }),
            Some(TransmitWithCsmaCaResult::CsmaCaBackoff(nb)),
        )
    }
}

impl<'svc, RadioDriverImpl: DriverConfig> ServiceTask<'svc, SchedulerService<'svc, RadioDriverImpl>>
    for TransmitWithCsmaCaTask<'svc, RadioDriverImpl>
{
    type Request = MpduFrame;
    type Result = TransmitWithCsmaCaResult;
    type Error = TransmitWithCsmaCaError;

    fn step(
        mut self,
        event: ServiceTaskEvent<'svc, SchedulerService<'svc, RadioDriverImpl>>,
    ) -> ServiceTaskTransition<'svc, SchedulerService<'svc, RadioDriverImpl>, Self> {
        match self.state {
            TransmitWithCsmaCaState::Initial(radio_frame, context) => {
                debug_assert!(matches!(event, ServiceTaskEvent::Entry));
                let min_be = { context.borrow().pib.min_be };
                let anchor_time = TransmitWithCsmaCaTask::anchor_time(context)
                    .checked_add_duration(TransmitWithCsmaCaTask::next_backoff_duration(
                        context, min_be,
                    ))
                    .unwrap();
                self.state = TransmitWithCsmaCaState::AttemptingTransmission(
                    0,
                    min_be,
                    anchor_time,
                    context,
                );
                ServiceTaskTransition::Intermediate(
                    self,
                    DrvSvcRequest::Tx(TaskTx {
                        at: Timestamp::Scheduled(anchor_time),
                        radio_frame,
                        cca: true,
                    }),
                    None,
                )
            }
            TransmitWithCsmaCaState::AttemptingTransmission(nb, be, anchor_time, context) => {
                match event {
                    ServiceTaskEvent::LowerServiceResponse(DrvSvcResponse::Tx(tx_result)) => {
                        match tx_result {
                            Ok(TxResult::Sent(radio_frame)) => {
                                #[cfg(feature = "rtos-trace")]
                                rtos_trace::trace::marker(TX_FRAME);

                                ServiceTaskTransition::Terminated(Ok(
                                    TransmitWithCsmaCaResult::Sent(radio_frame, nb),
                                ))
                            }
                            Ok(TxResult::Nack(unacknowledged_tx_frame)) => {
                                #[cfg(feature = "rtos-trace")]
                                rtos_trace::trace::marker(TX_NACK);

                                ServiceTaskTransition::Terminated(Ok(
                                    TransmitWithCsmaCaResult::NoAck(unacknowledged_tx_frame),
                                ))
                            }
                            Err(tx_error) => {
                                self.handle_tx_error(tx_error, nb, be, anchor_time, context)
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;
    use dot15d4_driver::constants::{MAC_UNIT_BACKOFF_PERIOD, PHY_CCA_DURATION};
    use dot15d4_driver::radio::DriverConfig;
    use dot15d4_driver::tasks::Timestamp;
    use dot15d4_driver::timer::RadioTimerApi;
    use dot15d4_util::allocator::IntoBuffer;

    use crate::mac::csma::{TransmitWithCsmaCaResult, SERVICE_GUARD_TIME};
    use crate::mac::pib::Pib;
    use crate::mac::tests::{
        generate_data_frame, FakeDriverConfig, FakeRadioTimer, FakeRng, TaskTestEvent,
        TaskTestTransition, TaskTester,
    };
    use crate::mac::MacContext;

    use super::TransmitWithCsmaCaTask;

    #[test]
    fn csma_ca_no_backoff_success() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });
        let task = TransmitWithCsmaCaTask::<FakeDriverConfig>::new(radio_frame, &context);

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(SERVICE_GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(FakeDriverConfig::GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[0] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxSent,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmitWithCsmaCaResult::Sent(radio_frame, _) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => panic!("Unexpected CSMA/CA result"),
            }),
        );
    }

    #[test]
    fn csma_ca_no_backoff_failure() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });

        context.borrow_mut().pib.max_csma_backoffs = 1;

        let task = TransmitWithCsmaCaTask::<FakeDriverConfig>::new(radio_frame, &context);

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(SERVICE_GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(FakeDriverConfig::GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[0] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[1] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmitWithCsmaCaResult::ChannelAccessFailure(radio_frame) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => panic!("Unexpected CSMA/CA result"),
            }),
        );
    }

    #[test]
    fn csma_ca_no_backoff_scheduling_error_reset() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });

        context.borrow_mut().pib.max_csma_backoffs = 1;

        let task = TransmitWithCsmaCaTask::<FakeDriverConfig>::new(radio_frame, &context);

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(SERVICE_GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(FakeDriverConfig::GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[0] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxSchedulingError,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(SERVICE_GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(FakeDriverConfig::GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[1] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxSent,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmitWithCsmaCaResult::Sent(radio_frame, _) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => panic!("Unexpected CSMA/CA result"),
            }),
        );
    }

    #[test]
    fn csma_ca_no_backoff_scheduling_error_cca() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });

        context.borrow_mut().pib.max_csma_backoffs = 2;

        let task = TransmitWithCsmaCaTask::<FakeDriverConfig>::new(radio_frame, &context);

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(SERVICE_GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(FakeDriverConfig::GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[0] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[1] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxSchedulingError,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[2] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxSent,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmitWithCsmaCaResult::Sent(radio_frame, _) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => panic!("Unexpected CSMA/CA result"),
            }),
        );
    }

    #[test]
    fn csma_ca_no_backoff_noack() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });

        let task = TransmitWithCsmaCaTask::<FakeDriverConfig>::new(radio_frame, &context);

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(SERVICE_GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(FakeDriverConfig::GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[0] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxNoAck,
            TaskTestTransition::TaskTerminated(&|result| {
                // TODO check timestamps
                match result {
                    TransmitWithCsmaCaResult::NoAck(radio_frame) => {
                        unsafe { radio_frame.into_buffer().consume() };
                    }
                    _ => panic!("Unexpected CSMA/CA result"),
                }
            }),
        );
    }

    #[test]
    fn csma_ca_single_backoff() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [2, 1, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });

        let task = TransmitWithCsmaCaTask::<FakeDriverConfig>::new(radio_frame, &context);

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(SERVICE_GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(FakeDriverConfig::GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[0] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[1] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                assert_eq!(result.unwrap(), TransmitWithCsmaCaResult::CsmaCaBackoff(1));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxSent,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmitWithCsmaCaResult::Sent(radio_frame, _) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => panic!("Unexpected CSMA/CA result"),
            }),
        );
    }

    #[test]
    fn csma_ca_max_backoffs() {
        // Allocating non-droppable buffer
        const BUF_LEN: usize = 127;
        static mut BUFFER: [u8; BUF_LEN] = [0; BUF_LEN];
        // Dataframe used by the state machine
        #[allow(static_mut_refs)]
        let radio_frame = unsafe { generate_data_frame(&mut BUFFER) };

        // Fake RNG with predefined sequence of numbers
        let arbitrary_sequence = [1, 2, 4, 0];
        let mut rng = FakeRng::new(&arbitrary_sequence);

        let context = RefCell::new(MacContext {
            pib: Pib::default(),
            rng: &mut rng,
            timer: FakeRadioTimer::new(),
        });

        context.borrow_mut().pib.max_csma_backoffs = 1;

        let task = TransmitWithCsmaCaTask::<FakeDriverConfig>::new(radio_frame, &context);

        let mut tester = TaskTester::new(task);

        tester.assert_transition(
            TaskTestEvent::TaskEntry,
            TaskTestTransition::DrvReqTx(&|task_tx, _result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(SERVICE_GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(FakeDriverConfig::GUARD_TIME)
                    .unwrap()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[0] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::DrvReqTx(&|task_tx, result| {
                let expected_timestamp = context
                    .borrow()
                    .timer
                    .now()
                    .checked_add_duration(MAC_UNIT_BACKOFF_PERIOD * (arbitrary_sequence[1] as u32))
                    .unwrap()
                    .checked_add_duration(PHY_CCA_DURATION)
                    .unwrap();
                assert_eq!(task_tx.at, Timestamp::Scheduled(expected_timestamp));
                assert_eq!(result.unwrap(), TransmitWithCsmaCaResult::CsmaCaBackoff(1));
                context.borrow_mut().timer.update(expected_timestamp);
                task_tx
            }),
        );

        tester.assert_transition(
            TaskTestEvent::DrvRespTxCcaBusy,
            TaskTestTransition::TaskTerminated(&|result| match result {
                TransmitWithCsmaCaResult::ChannelAccessFailure(radio_frame) => {
                    unsafe { radio_frame.into_buffer().consume() };
                }
                _ => panic!("Unexpected CSMA/CA result"),
            }),
        );
    }
}
