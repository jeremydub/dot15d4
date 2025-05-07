use dot15d4_frame3::driver::Tx;
use dot15d4_frame3::mpdu::MpduFrame;

use super::mcps::data::DataIndication;
pub use super::mcps::data::DataRequest;
use super::mlme::beacon::{BeaconNotifyIndication, BeaconRequest};
use super::mlme::set::SetRequestAttribute;

/// Enum representing all (currently) supported MAC services request primitives
pub enum MacRequest<'mpdu> {
    /// IEEE 802.15.4-2020, section 8.2.6.4
    MlmeSetRequest(SetRequestAttribute),
    /// IEEE 802.15.4-2020, section 8.2.18.1
    MlmeBeaconRequest(BeaconRequest),
    /// IEEE 802.15.4-2020, section 8.3.2
    McpsDataRequest(DataRequest<'mpdu>),
}

impl<'mpdu> MacRequest<'mpdu> {
    fn new(mpdu: MpduFrame<'mpdu, Tx>) -> Self {
        Self::McpsDataRequest(DataRequest { mpdu })
    }
}

pub enum MacIndication<'mpdu> {
    McpsData(DataIndication<'mpdu>),
    MlmeBeaconNotify(BeaconNotifyIndication<'mpdu>),
}
