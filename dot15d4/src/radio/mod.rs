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
use generic_array::GenericArray;

use crate::{
    select::select,
    sync::{
        channel::{Receiver, Sender},
        yield_now::yield_now,
        Either,
    },
};

use self::config::{RxConfig, TxConfig};

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

/// Structure managing the driver. Knows about and manages driver capabilities
/// and exposes a unified API to the MAC service.
pub struct DriverCoprocessor<'radio, Config: DriverConfig, R: RadioDriver<Config>> {
    radio: RefCell<R>,
    tx_recv: Receiver<'radio, DriverFrame<'radio, Config, Tx>>, // TODO: Fix Tx buffer lifetime.
    rx_buf: GenericArray<u8, Config::MaxFrameLen>,
    rx_send: Sender<'radio, DriverFrame<'radio, Config, Rx>>, // TODO: Fix Rx buffer lifetime.
    /// PAN Information Base
    pub pib: pib::Pib,
    driver_config: PhantomData<Config>,
}

impl<'radio, Config: DriverConfig, R: RadioDriver<Config>> DriverCoprocessor<'radio, Config, R> {
    /// Creates a new [`PhyService<Config, R>`].
    pub fn new(
        radio: R,
        tx_recv: Receiver<'radio, DriverFrame<'radio, Config, Tx>>,
        rx_send: Sender<'radio, DriverFrame<'radio, Config, Rx>>,
    ) -> Self {
        Self {
            radio: RefCell::new(radio),
            tx_recv,
            rx_buf: Default::default(),
            rx_send,
            pib: pib::Pib::default(),
            driver_config: PhantomData,
        }
    }

    /// Run the main event loop used by the PHY sublayer for its operation. For
    /// now, the loop waits for either receiving a frame from the MAC sublayer
    /// or receiving a frame from the radio.
    pub async fn run(&mut self) -> ! {
        self.radio.borrow_mut().enable().await; // Wake up radio

        loop {
            yield_now().await;

            // TODO: Describe, analyze, optimize and measure alternative
            //       allocation strategies (stack, heap, static, object
            //       allocator, ...).
            let mut driver_frame = DriverFrame::new(&mut self.rx_buf);

            match select(self.rx_frame(&mut driver_frame), self.mac_recv()).await {
                Either::First(_) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(PHY_RX);
                    self.mac_send(driver_frame).await;
                }
                Either::Second(mut tx_frame) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(PHY_RX);
                    self.tx_frame(&mut tx_frame).await;
                }
            };
        }
        //
    }

    /// Send a frame back to the MAC sublayer.
    async fn mac_send(&self, rx: DriverFrame<'radio, Config, Rx>) {
        self.rx_send.send_async(rx).await;
    }

    /// Wait for a frame from the MAC sublayer to be transmitted.
    async fn mac_recv(&self) -> DriverFrame<'radio, Config, Tx> {
        self.tx_recv.receive().await
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
