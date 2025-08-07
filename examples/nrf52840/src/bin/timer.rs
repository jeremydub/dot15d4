#![no_std]
#![no_main]

use panic_probe as _;

use dot15d4_driver::{
    executor::InterruptExecutor,
    socs::nrf::{executor, NrfRadioTimer},
    timer::{HardwareSignal, Pin, RadioTimerApi, RadioTimerResult, SyntonizedDuration},
};
#[cfg(feature = "gpio-trace")]
use dot15d4_examples_nrf52840::PIN_EXECUTOR;
use dot15d4_examples_nrf52840::{config_peripherals, toggle_gpiote_pin, PIN_ALARM};
use embassy_executor::Spawner;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let (_, peripherals) = config_peripherals();

    let toggle_alarm_pin = || {
        toggle_gpiote_pin(&peripherals.gpiote, PIN_ALARM.gpiote_channel as usize);
    };

    #[cfg(feature = "gpio-trace")]
    let gpiote_trace_channel = PIN_EXECUTOR.gpiote_channel as usize;
    let executor = executor::swi0(
        peripherals.swi0,
        #[cfg(feature = "gpio-trace")]
        &peripherals.gpiote,
        #[cfg(feature = "gpio-trace")]
        gpiote_trace_channel,
    );

    let timer_task = async {
        let mut timeout = NrfRadioTimer::now();
        for _ in 0..10 {
            const DELAY: SyntonizedDuration = SyntonizedDuration::nanos(4 * 30518);
            timeout += DELAY;

            // Safety: We run at lower priority than the timer interrupt and we
            //         run from a single task.
            let result = unsafe {
                NrfRadioTimer::schedule_event(timeout, HardwareSignal::TogglePin(Pin::Pin0)).await
            };
            // let result = unsafe { NrfRadioTimer::wait_until(timeout).await };
            assert!(matches!(result, RadioTimerResult::Ok));
        }
        toggle_alarm_pin();
    };
    unsafe {
        executor.spawn(timer_task).await;
    }

    toggle_alarm_pin();

    // # MAC service (running on SWI5 executor)
    //
    // Race for:
    // - a request to schedule a frame
    // - timeout of queue head timer (or "never", if the queue is empty)
    // On frame:
    // - Find an adequate slot for the frame.
    // - Calculate/check the timing of the frame.
    // - Push the frame into the queue.
    // - Calculate the timer for head with sufficient guard time (max(schedule frame, schedule event).
    // On timeout:
    // - Pop head from the queue.
    // - Send head to the driver service.

    // # Driver service (running on SWI1 executor)
    //
    // - Wait for request to schedule an event.
    // - Schedule event to timer.

    #[cfg(feature = "rtos-trace")]
    rtos_trace::trace::stop();
}
