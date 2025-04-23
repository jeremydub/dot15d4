pub mod acknowledgment;
pub mod constants;
pub mod mcps;
pub mod mlme;
pub mod neighbors;
pub mod pib;
pub mod primitives;
pub mod tsch;
pub mod utils;

use crate::{
    sync::{
        channel::{Receiver, Sender},
        join,
        mutex::Mutex,
        select,
        yield_now::yield_now,
        Either,
    },
    upper::UpperLayer,
};
use dot15d4_frame3::{
    driver::{DriverConfig, DriverFrame, Rx, Tx},
    frame_control::FrameType,
    mpdu::{MpduFrame, IMM_ACK_BUF_LEN},
};
use embedded_hal_async::delay::DelayNs;
use mcps::data::DataIndication;
use mlme::beacon::BeaconNotifyIndication;
use rand_core::RngCore;

#[cfg(feature = "rtos-trace")]
use crate::trace::{MAC_INDICATION, MAC_REQUEST};

pub use primitives::{MacIndication, MacRequest};

/// MAC-related error propagated to higher layer
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Error {
    /// Cca failed, resulting in a backoff (nth try)
    CcaBackoff(u8),
    /// Cca failed after to many fallbacks
    CcaFailed,
    /// Ack failed, resulting in a retry later (nth try)
    AckRetry(u8),
    /// Ack failed, after to many retransmissions
    AckFailed,
    /// The buffer did not follow the correct device structure
    InvalidDeviceStructure,
    /// Invalid IEEE frame
    InvalidIEEEStructure,
    /// Something went wrong
    Error,
}

#[allow(dead_code)]
/// Structure handling MAC sublayer services such as MLME and MCPS. This runs the main event loop
/// that handles interactions between an upper layer and the PHY sublayer. It uses signals to
/// communicate with the upper layer and with the PHY sublayer.
pub struct MacService<'svc, Rng, U: UpperLayer, TIMER, Config: DriverConfig> {
    /// Pseudo-random number generator
    rng: &'svc mut Mutex<Rng>,
    /// Timer enabling delays operation
    timer: TIMER,
    /// Upper layer handler from which MAC commands are received and to which
    /// frames and responses are passed.
    upper_layer: U,
    /// Signal to receive a primitive from the upper layer
    rx_recv: Receiver<'svc, DriverFrame<'svc, Config, Rx>>, // TODO: Fix buffer lifetime.
    /// Signal for sending a frame to the PHY sublayer
    tx_send: Sender<'svc, DriverFrame<'svc, Config, Tx>>, // TODO: Fix buffer lifetime.
    // TODO: Move to mutable "inner". Make inner a state machine?
    /// PAN Information Base
    pub pib: pib::Pib,
}

impl<'svc, Rng, U, TIMER, Config> MacService<'svc, Rng, U, TIMER>
where
    Rng: RngCore,
    U: UpperLayer,
    Config: DriverConfig,
{
    /// Creates a new [`MacService<Rng, U, TIMER, R>`].
    pub fn new(
        rng: &'svc mut Mutex<Rng>,
        upper_layer: U,
        timer: TIMER,
        rx_recv: Receiver<'svc, DriverFrame<'svc, Config, Rx>>,
        tx_send: Sender<'svc, DriverFrame<'svc, Config, Tx>>,
    ) -> Self {
        Self {
            rng,
            upper_layer,
            timer,
            rx_recv,
            tx_send,
            pib: pib::Pib::default(),
        }
    }
}

#[allow(dead_code)]
impl<'svc, Rng, U, TIMER, Config> MacService<'svc, Rng, U, TIMER>
where
    Rng: RngCore,
    U: UpperLayer,
    TIMER: DelayNs + Clone,
    Config: DriverConfig,
{
    /// Run the main event loop used by the MAC sublayer for its operation. For
    /// now, the loop waits for either receiving a command from the upper layer
    /// or a frame/indication from the PHY sublayer.
    pub async fn run(&mut self) -> ! {
        loop {
            yield_now().await;
            // Wait until we either have a command to process from the upper layer or we
            // receive an indication from the PHY sublayer
            match select::select(self.upper_layer.mac_request(), self.receive_indication()).await {
                Either::First(request) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(MAC_REQUEST);
                    self.handle_request(request).await;
                }
                Either::Second(Some(indication)) => {
                    #[cfg(feature = "rtos-trace")]
                    rtos_trace::trace::task_exec_begin(MAC_INDICATION);
                    self.handle_indication(indication).await;
                }
                _ => {}
            };
        }
    }

    /// Submit a buffer to the PHY sublayer via a signal that is received by
    /// the PHY task. Wait for the frame to be fully transmitted before
    /// returning.
    async fn phy_send(&self, tx: DriverFrame<'svc, Config, Tx>) {
        self.tx_send.send_async(tx).await;
    }

    /// Waits for a frame to be received from the PHY sublayer's task via a
    /// signal.
    async fn phy_receive(&self) -> DriverFrame<'svc, Config, Rx> {
        self.rx_recv.receive().await
    }

    async fn receive_indication(&self) -> Option<MacIndication<'svc>> {
        static mut ACK_BUFFER: [u8; IMM_ACK_BUF_LEN] = [0; IMM_ACK_BUF_LEN];

        let frame = self.phy_receive().await;
        let mut mpdu = MpduFrame::parse(frame);

        // Optional ack frame that is used if required
        // SAFETY: We prepare and transmit the ack frame sequentially from a
        //         single executor.
        let ack_mpdu = unsafe { self.prepare_ack(&mut mpdu, &mut ACK_BUFFER) };

        // Acknowledgment is sent while the indication is processed
        let (_, indication) = join::join(self.transmit_ack(ack_mpdu), async {
            let frame_type = mpdu.frame_control().frame_type();
            // TODO: support timestamp
            let timestamp = 0;
            match frame_type {
                FrameType::Data => {
                    Some(MacIndication::McpsData(DataIndication { mpdu, timestamp }))
                }
                FrameType::Beacon => {
                    Some(MacIndication::MlmeBeaconNotify(BeaconNotifyIndication {
                        mpdu,
                        timestamp,
                    }))
                }
                _ => None,
            }
        })
        .await;

        indication
    }

    // TODO: Move to mutable "inner".
    async fn handle_indication(&self, indication: MacIndication<'svc>) {
        match indication {
            MacIndication::McpsData(data_indication) => {
                self.mcps_data_indication(data_indication).await;
            }
            MacIndication::MlmeBeaconNotify(beacon_notify_indication) => {
                self.mlme_beacon_notify_indication(beacon_notify_indication)
                    .await;
            }
        }
    }

    // TODO: Move to mutable "inner".
    async fn handle_request(&mut self, request: MacRequest<'svc>) {
        match request {
            MacRequest::McpsDataRequest(request) => {
                // TODO: handle errors with upper layer
                let _ = self.mcps_data_request(request.mpdu).await;
            }
            MacRequest::MlmeBeaconRequest(beacon_request) => {
                // TODO: handle errors with upper layer
                let _ = self.mlme_beacon_request(&beacon_request).await;
            }
            MacRequest::MlmeSetRequest(set_request_attribute) => {
                // TODO: handle errors with upper layer
                let _ = self.mlme_set_request(&set_request_attribute).await;
            }
        }
    }
}
