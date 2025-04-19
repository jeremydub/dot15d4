use typenum::U;

use crate::driver::{DriverConfig, DriverFrameRepr, FcsNone, MAX_PHY_PACKET_SIZE_127};

#[derive(Clone, Copy, Debug)]
pub struct NrfDriverConfig;

const PHY_HDR_LEN: usize = 1;
pub const FCS_LEN: usize = 2;

impl DriverConfig for NrfDriverConfig {
    type Headroom = U<PHY_HDR_LEN>; // Headroom for the PHY header (packet length).
    type Tailroom = U<FCS_LEN>; // Tailroom for driver-level FCS handling.
    type MaxFrameLen = U<{ PHY_HDR_LEN + MAX_PHY_PACKET_SIZE_127 + FCS_LEN }>; // The FCS is handled by the driver and must not be part of the MACs MPDU.
    type Fcs = FcsNone; // Assuming automatic FCS handling.
}

pub const DRIVER_OVERHEAD: usize = DriverFrameRepr::<NrfDriverConfig>::driver_overhead();
