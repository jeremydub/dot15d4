//! Access to IEEE 802.15.4 devices.
//!
//! This module provides access to IEEE 802.15.4 devices. It provides a trait
//! for transmitting and receiving frames, [Device].

pub mod config;
pub mod constants;
pub mod driver;
pub mod pib;

use core::{cell::RefCell, marker::PhantomData};

use dot15d4_frame3::driver::{DriverConfig, DriverFrame, Rx, Tx};
use driver::RadioDriver;

use crate::sync::channel::Receiver;

use self::config::{RxConfig, TxConfig};

/// Placeholder for future radio task abstraction.
enum DriverTask<'buffer, Config: DriverConfig> {
    TxFrame(DriverFrame<'buffer, Config, Tx>),
    RxFrame(DriverFrame<'buffer, Config, Rx>),
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Error {
    /// Ack failed, after too many retransmissions
    AckFailed,
    /// The buffer did not follow the correct device structure
    InvalidDeviceStructure,
    /// Something went wrong in the radio
    RadioError,
}

// TODO: Make this configurable.
pub const DRIVER_CHANNEL_CAPACITY: usize = 2;
pub const DRIVER_CHANNEL_BACKLOG: usize = 2;

/// Structure managing the driver. Knows about and manages driver capabilities
/// and exposes a unified API to the MAC service.
pub struct DriverCoprocessor<'radio, Config: DriverConfig, R: RadioDriver<Config>> {
    // TODO: Consolidate interior mutability in an inner struct.
    radio: RefCell<R>,
    mac: RefCell<
        Receiver<
            'radio,
            DriverTask<'radio, Config>,
            DRIVER_CHANNEL_CAPACITY,
            DRIVER_CHANNEL_BACKLOG,
        >,
    >,
    /// PAN Information Base
    pub pib: pib::Pib,
    driver_config: PhantomData<Config>,
}

impl<'radio, Config: DriverConfig, R: RadioDriver<Config>> DriverCoprocessor<'radio, Config, R> {
    /// Creates a new [`PhyService<Config, R>`].
    pub fn new(
        radio: R,
        mac: Receiver<
            'radio,
            DriverTask<'radio, Config>,
            DRIVER_CHANNEL_CAPACITY,
            DRIVER_CHANNEL_BACKLOG,
        >,
    ) -> Self {
        Self {
            radio: RefCell::new(radio),
            mac: RefCell::new(mac),
            pib: pib::Pib::default(),
            driver_config: PhantomData,
        }
    }

    /// Run the main event loop used by the PHY sublayer for its operation. For
    /// now, the loop waits for either receiving a frame from the MAC sublayer
    /// or receiving a frame from the radio.
    pub async fn run(&self) -> ! {
        self.radio.borrow_mut().enable().await; // Wake up radio

        let mut mac = self.mac.borrow_mut();
        loop {
            let (slot, msg) = mac.wait_for_msg().await;
            match msg {
                DriverTask::TxFrame(mut tx_frame) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(PHY_RX);
                    self.tx_frame(&mut tx_frame).await;
                }
                DriverTask::RxFrame(mut rx_frame) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(PHY_TX);
                    self.rx_frame(&mut rx_frame).await;
                }
            }
            mac.received(slot);
        }
        //
    }

    /// Listen for a frame on the radio
    async fn rx_frame(&self, frame: &mut DriverFrame<'_, Config, Rx>) -> Result<(), ()> {
        self.radio
            .borrow_mut()
            .receive(
                RxConfig {
                    channel: self.pib.current_channel.try_into().unwrap(),
                },
                frame,
            )
            .await
    }

    /// Transmit the given frame to the radio
    async fn tx_frame(&self, frame: &mut DriverFrame<'_, Config, Tx>) {
        self.radio
            .borrow_mut()
            .transmit(
                TxConfig {
                    channel: self.pib.current_channel.try_into().unwrap(),
                    ..Default::default()
                },
                frame,
            )
            .await;
    }
}
