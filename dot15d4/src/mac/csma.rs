#![allow(dead_code)]
use core::{cell::RefCell, cmp::min, marker::PhantomData};

use dot15d4_driver::{
    constants::{MAC_UNIT_BACKOFF_PERIOD, PHY_CCA_DURATION},
    frame::{RadioFrame, RadioFrameSized},
    radio::DriverConfig,
    timer::{LocalClockDuration, LocalClockInstant, RadioTimerApi},
};
use typenum::Unsigned;

use crate::{
    driver::{
        tasks::{Timestamp, TxError, TxResult},
        DrvSvcResponse, DrvSvcTaskError, DrvSvcTaskTx,
    },
    mac::task::{MacTaskEvent, MacTaskTransition},
};

#[cfg(feature = "rtos-trace")]
use crate::trace::{TX_CCABUSY, TX_FRAME, TX_NACK};

use super::{task::MacTask, MacSvcContext};

struct UnslottedCsmaCa;
struct SlottedCsmaCa;
struct TschCsmaCa;

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
    /// Failed after maximum number of attempts.
    ///
    /// This is always a final result.
    ChannelAccessFailure(RadioFrame<RadioFrameSized>),
}

pub(crate) enum CsmaCaState<'task, RadioDriverImpl: DriverConfig> {
    Initial(
        /// Radio frame to be sent.
        RadioFrame<RadioFrameSized>,
        /// MAC Service context
        &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
    ),
    AttemptingTransmission(
        u8,
        u8,
        LocalClockInstant,
        &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
    ),
}

pub(crate) struct TransmitWithCsmaCaTask<'task, RadioDriverImpl: DriverConfig> {
    state: CsmaCaState<'task, RadioDriverImpl>,
    radio: PhantomData<RadioDriverImpl>,
}

impl<'task, RadioDriverImpl: DriverConfig> TransmitWithCsmaCaTask<'task, RadioDriverImpl> {
    pub(super) fn new(
        radio_frame: RadioFrame<RadioFrameSized>,
        context: &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
    ) -> Self {
        Self {
            state: CsmaCaState::Initial(radio_frame, context),
            radio: PhantomData,
        }
    }

    fn next_backoff_duration(
        context: &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
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
    fn anchor_time(
        context: &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
    ) -> LocalClockInstant {
        // Time necessary by the driver to go from any state to Tx Ready
        // (including CCA and turnaround)
        let driver_guard_time =
            LocalClockDuration::micros(<RadioDriverImpl::TxGuardTime as Unsigned>::to_u64());
        // TODO: make it configurable
        // Delay induced by the CPU for processing the scheduling event in async executor
        const SCHEDULING_GUARD_TIME: LocalClockDuration = LocalClockDuration::micros(500);

        context
            .borrow()
            .timer
            .now()
            .checked_add_duration(driver_guard_time)
            .unwrap()
            .checked_add_duration(SCHEDULING_GUARD_TIME)
            .unwrap()
    }

    fn handle_tx_error(
        mut self,
        tx_error: DrvSvcTaskError<DrvSvcTaskTx>,
        nb: u8,
        be: u8,
        anchor_time: LocalClockInstant,
        context: &'task RefCell<MacSvcContext<'task, RadioDriverImpl>>,
    ) -> MacTaskTransition<TransmitWithCsmaCaTask<'task, RadioDriverImpl>> {
        let next_be = min(be + 1, context.borrow().pib.max_be);
        let mut nb = nb;
        // Anchor time to use for next attempt depending on the TX error, and recovered radio frame
        let (next_anchor_time, radio_frame) = match tx_error {
            DrvSvcTaskError::Task(TxError::CcaBusy(radio_frame)) => {
                #[cfg(feature = "rtos-trace")]
                rtos_trace::trace::marker(TX_CCABUSY);
                nb += 1;
                (
                    anchor_time + TransmitWithCsmaCaTask::next_backoff_duration(context, next_be),
                    radio_frame,
                )
            }
            DrvSvcTaskError::SchedulingError(task) => {
                if nb == 1 {
                    (
                        TransmitWithCsmaCaTask::anchor_time(context),
                        task.radio_frame,
                    )
                } else {
                    nb += 1;
                    (
                        anchor_time
                            + TransmitWithCsmaCaTask::next_backoff_duration(context, next_be),
                        task.radio_frame,
                    )
                }
            }
        };
        if nb > context.borrow().pib.max_csma_backoffs {
            return MacTaskTransition::Terminated(TransmitWithCsmaCaResult::ChannelAccessFailure(
                radio_frame,
            ));
        }

        self.state = CsmaCaState::AttemptingTransmission(nb, next_be, next_anchor_time, context);
        MacTaskTransition::DrvSvcRequest(
            self,
            DrvSvcTaskTx {
                at: Timestamp::Scheduled(next_anchor_time),
                radio_frame,
                cca: true,
            }
            .into(),
            Some(TransmitWithCsmaCaResult::CsmaCaBackoff(nb)),
        )
    }
}

impl<'task, RadioDriverImpl: DriverConfig> MacTask
    for TransmitWithCsmaCaTask<'task, RadioDriverImpl>
{
    type Result = TransmitWithCsmaCaResult;

    fn step(mut self, event: MacTaskEvent) -> MacTaskTransition<Self> {
        match self.state {
            CsmaCaState::Initial(radio_frame, context) => {
                debug_assert!(matches!(event, MacTaskEvent::Entry));
                let min_be = { context.borrow().pib.min_be };
                let anchor_time = TransmitWithCsmaCaTask::anchor_time(context);
                self.state = CsmaCaState::AttemptingTransmission(1, min_be, anchor_time, context);
                MacTaskTransition::DrvSvcRequest(
                    self,
                    DrvSvcTaskTx {
                        at: Timestamp::Scheduled(anchor_time),
                        radio_frame,
                        cca: true,
                    }
                    .into(),
                    None,
                )
            }
            CsmaCaState::AttemptingTransmission(nb, be, anchor_time, context) => match event {
                MacTaskEvent::DrvSvcResponse(DrvSvcResponse::Tx(tx_result)) => match tx_result {
                    Ok(TxResult::Sent(radio_frame)) => {
                        #[cfg(feature = "rtos-trace")]
                        rtos_trace::trace::marker(TX_FRAME);

                        MacTaskTransition::Terminated(TransmitWithCsmaCaResult::Sent(
                            radio_frame,
                            nb,
                        ))
                    }
                    Ok(TxResult::Nack(unacknowledged_tx_frame)) => {
                        #[cfg(feature = "rtos-trace")]
                        rtos_trace::trace::marker(TX_NACK);

                        MacTaskTransition::Terminated(TransmitWithCsmaCaResult::NoAck(
                            unacknowledged_tx_frame,
                        ))
                    }
                    Err(tx_error) => self.handle_tx_error(tx_error, nb, be, anchor_time, context),
                },
                _ => unreachable!(),
            },
        }
    }
}
