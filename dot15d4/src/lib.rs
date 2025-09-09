#![cfg_attr(not(feature = "std"), no_std)]
mod driver;
pub mod mac;
mod pib;
mod scheduler;
mod service;

use core::cell::RefCell;

pub use dot15d4_util as util;
use rand_core::RngCore;

use self::{
    driver::{
        radio::{DriverConfig, RadioDriver, RadioDriverApi},
        tasks::{
            OffState, RxState, TaskOff as RadioTaskOff, TaskRx as RadioTaskRx,
            TaskTx as RadioTaskTx, TxState,
        },
        DriverRequestChannel, DriverService,
    },
    mac::{MacBufferAllocator, MacRequestReceiver, MacService},
    pib::Pib,
    scheduler::{SchedulerRequestChannel, SchedulerService},
    util::sync::{select, Either},
};

pub(crate) struct MacContext<'upper_layer, RadioDriverImpl: DriverConfig> {
    pib: Pib,
    rng: &'upper_layer mut dyn RngCore,
    timer: RadioDriverImpl::Timer,
}

pub struct Device<RadioDriverImpl: DriverConfig> {
    radio: RadioDriver<RadioDriverImpl, RadioTaskOff>,
}

impl<RadioDriverImpl: DriverConfig> Device<RadioDriverImpl> {
    pub fn new(radio: RadioDriver<RadioDriverImpl, RadioTaskOff>) -> Self {
        Self { radio }
    }
}

impl<RadioDriverImpl: DriverConfig> Device<RadioDriverImpl>
where
    RadioDriver<RadioDriverImpl, RadioTaskOff>: OffState<RadioDriverImpl> + RadioDriverApi,
    RadioDriver<RadioDriverImpl, RadioTaskRx>: RxState<RadioDriverImpl> + RadioDriverApi,
    RadioDriver<RadioDriverImpl, RadioTaskTx>: TxState<RadioDriverImpl> + RadioDriverApi,
{
    pub async fn run<'upper_layer>(
        self,
        buffer_allocator: MacBufferAllocator,
        mac_request_receiver: MacRequestReceiver<'upper_layer, RadioDriverImpl>,
        timer: RadioDriverImpl::Timer,
        rng: &'upper_layer mut dyn RngCore,
    ) -> ! {
        #[cfg(feature = "rtos-trace")]
        self::trace::instrument();

        let context = RefCell::new(MacContext {
            pib: Pib::default(),
            rng,
            timer,
        });

        let scheduler_service_channel = SchedulerRequestChannel::new();
        let driver_service_channel = DriverRequestChannel::new();

        let mut mac_service = MacService::<RadioDriverImpl>::new(
            &context,
            buffer_allocator,
            mac_request_receiver,
            scheduler_service_channel.sender(),
        );
        let mut scheduler_service = SchedulerService::<'upper_layer, RadioDriverImpl>::new(
            timer,
            &context,
            buffer_allocator,
            scheduler_service_channel.receiver(),
            driver_service_channel.sender(),
        );
        let driver_service = DriverService::new(
            self.radio,
            driver_service_channel.receiver(),
            buffer_allocator,
        );

        match select::select(
            select::select(mac_service.run(), scheduler_service.run()),
            driver_service.run(),
        )
        .await
        {
            Either::First(either) => match either {
                Either::First(_) => panic!("MAC service terminated"),
                Either::Second(_) => panic!("MAC Scheduler service terminated"),
            },
            Either::Second(_) => panic!("Driver service terminated"),
        }
    }
}

#[cfg(feature = "rtos-trace")]
pub mod trace {
    use crate::util::trace::TraceOffset;

    #[cfg(feature = "defmt")]
    compile_error!(
        "Tracing cannot be enabled at the same time as defmt. Logs will be visible in the SystemView application if the 'log' feature is enabled."
    );

    const OFFSET: TraceOffset = TraceOffset::Dot15d4;

    // Tasks
    pub const MAC_INDICATION: u32 = OFFSET.wrap(0);
    pub const MAC_REQUEST: u32 = OFFSET.wrap(1);

    // Markers
    pub const TX_FRAME: u32 = OFFSET.wrap(0);
    pub const TX_NACK: u32 = OFFSET.wrap(1);
    pub const TX_CCABUSY: u32 = OFFSET.wrap(2);
    pub const RX_FRAME: u32 = OFFSET.wrap(3);
    pub const RX_INVALID: u32 = OFFSET.wrap(4);
    pub const RX_CRC_ERROR: u32 = OFFSET.wrap(5);
    pub const RX_WINDOW_ENDED: u32 = OFFSET.wrap(6);

    /// Instrument the library for tracing.
    pub(crate) fn instrument() {
        rtos_trace::trace::task_new_stackless(MAC_INDICATION, "MAC indication\0", 0);
        rtos_trace::trace::task_new_stackless(MAC_REQUEST, "MAC request\0", 0);
        rtos_trace::trace::name_marker(TX_FRAME, "TX frame\0");
        rtos_trace::trace::name_marker(TX_NACK, "TX NACK\0");
        rtos_trace::trace::name_marker(TX_CCABUSY, "TX CCA Busy\0");
        rtos_trace::trace::name_marker(RX_FRAME, "RX frame\0");
        rtos_trace::trace::name_marker(RX_INVALID, "RX invalid frame\0");
        rtos_trace::trace::name_marker(RX_CRC_ERROR, "RX CRC error\0");
        rtos_trace::trace::name_marker(RX_WINDOW_ENDED, "RX window ended\0");
    }
}
