use crate::upper::UpperLayer;
use dot15d4_frame3::{driver::Rx, mpdu::MpduFrame};
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

use super::MacService;

pub struct BeaconRequest {}

pub struct BeaconConfirm {}

pub struct BeaconNotifyIndication<'mpdu> {
    /// buffer containing the received frame payload
    pub mpdu: MpduFrame<'mpdu, Rx>,
    /// Timestamp of frame reception
    pub timestamp: u32,
}

#[allow(dead_code)]
impl<'svc, Rng, U, TIMER> MacService<'svc, Rng, U, TIMER>
where
    Rng: RngCore,
    U: UpperLayer,
    TIMER: DelayNs + Clone,
{
    /// Requests the generation of a Beacon frame or Enhanced Beacon frame.
    pub(crate) async fn mlme_beacon_request(
        &self,
        _request: &BeaconRequest,
    ) -> Result<BeaconConfirm, ()> {
        // TODO: fill with correct values
        let frame_repr = FrameBuilder::new_beacon_request()
            .finalize()
            .expect("A simple beacon request should always be possible to build");
        frame_repr.emit(&mut DataFrame::new_unchecked(&mut tx.buffer));
        self.phy_send(tx).await;

        Err(())
    }

    pub(crate) async fn mlme_beacon_notify_indication(
        &self,
        _indication: BeaconNotifyIndication<'svc>,
    ) {
        // TODO: support Beacon Notify indication
        info!("Received Beacon Notification");
    }
}
