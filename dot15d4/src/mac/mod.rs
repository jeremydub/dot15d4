pub mod acknowledgment;
pub mod constants;
pub mod mcps;
pub mod mlme;
pub mod neighbors;
pub mod pib;
pub mod primitives;
pub mod tsch;
pub mod utils;

use core::cell::RefCell;

use crate::{
    radio::{DRIVER_CHANNEL_BACKLOG, DRIVER_CHANNEL_CAPACITY},
    sync::{
        channel::{Receiver, Sender},
        join,
        mutex::Mutex,
        select, Either,
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
use primitives::MacRequest;
use rand_core::RngCore;

#[cfg(feature = "rtos-trace")]
use crate::trace::{MAC_INDICATION, MAC_REQUEST};

pub use primitives::MacIndication;

pub enum MacMsg<'mpdu> {
    Request(MacRequest<'mpdu>),
    RxBuffer(MpduFrame<'mpdu, Rx>),
}

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

// TODO: Make this configurable.
pub const UL_CHANNEL_CAPACITY: usize = 2;
pub const UL_CHANNEL_BACKLOG: usize = 2;

#[allow(dead_code)]
/// Structure handling MAC sublayer services such as MLME and MCPS. This runs the main event loop
/// that handles interactions between an upper layer and the PHY sublayer. It uses signals to
/// communicate with the upper layer and with the PHY sublayer.
pub struct MacService<'svc, Rng, TIMER, Config: DriverConfig> {
    /// Pseudo-random number generator
    rng: &'svc mut Mutex<Rng>,
    /// Timer enabling delays operation
    timer: TIMER,
    /// Upper layer channel from which MAC requests and MAC indication
    /// allocations are received.
    upper_layer: Receiver<'svc, MacMsg<'svc>, UL_CHANNEL_CAPACITY, UL_CHANNEL_BACKLOG>,
    /// Channel to communicate with the driver subsystem.
    driver: Sender<
        'svc,
        DriverFrame<'svc, Config, Rx>,
        DRIVER_CHANNEL_CAPACITY,
        DRIVER_CHANNEL_BACKLOG,
    >,
    /// PAN Information Base
    pub pib: RefCell<pib::Pib>,
}

impl<'svc, Rng, TIMER, Config> MacService<'svc, Rng, TIMER, Config>
where
    Rng: RngCore,
    Config: DriverConfig,
{
    /// Creates a new [`MacService<Rng, U, TIMER, R>`].
    pub fn new(
        rng: &'svc mut Mutex<Rng>,
        upper_layer: Receiver<'svc, MacMsg<'svc>, UL_CHANNEL_CAPACITY, UL_CHANNEL_BACKLOG>,
        timer: TIMER,
        driver: Sender<
            'svc,
            DriverFrame<'svc, Config, Tx>,
            DRIVER_CHANNEL_CAPACITY,
            DRIVER_CHANNEL_BACKLOG,
        >,
    ) -> Self {
        Self {
            rng,
            upper_layer,
            timer,
            driver,
            pib: RefCell::new(pib::Pib::default()),
        }
    }
}

#[allow(dead_code)]
impl<'svc, Rng, TIMER, Config> MacService<'svc, Rng, TIMER, Config>
where
    Rng: RngCore,
    TIMER: DelayNs + Clone,
    Config: DriverConfig,
{
    /// Run the main event loop used by the MAC sublayer for its operation. For
    /// now, the loop waits for either receiving a MCPS-DATA request from the
    /// upper layer or an allocated MCPS-DATA indication to wait for.
    pub async fn run(&mut self) -> ! {
        loop {
            let (slot, msg) = self.upper_layer.wait_for_msg().await;
            match msg {}
            // Wait until we either have a request to process from the upper layer or we
            // receive an indication from the PHY sublayer
            match select::select(self.upper_layer.mac_primitive(), self.receive_indication()).await
            {
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
    async fn handle_indication(&mut self, indication: MacIndication<'svc>) {
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
