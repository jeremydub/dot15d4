use core::marker::PhantomData;
use generic_array::ArrayLength;
use typenum::{Const, Sub1, ToUInt, Unsigned, U, U0, U127, U2048};

use crate::{SizeOf, StaticallySized};

// PHY
#[allow(dead_code)]
pub type MaxPhyPacketSize2047 = Sub1<U2048>; // SUN, TVWS, RCC, LECIM FSK, and MSK with a 2000 kb/s data rate
#[allow(dead_code)]
pub type MaxPhyPacketSize127 = U127; // all other PHYs

// types allowed for RadioDriverConfig::Fcs
// Drivers for LECIM, TVWS and SUN PHYs may be configured with a 4-byte FCS, all
// other drivers/PHYs use two bytes.
// Drivers that offload FCS (=CRC) checking to hardware will neither require
// nor include an FCS in the frame.
#[allow(dead_code)]
pub type Fcs2Byte = u16;

#[allow(dead_code)]
pub type Fcs4Byte = u32;

#[allow(dead_code)]
pub type NoFcs = ();

// Driver
pub trait RadioDriverConfig {
    /// HW-specific headroom required by the driver, e.g. buffer space for the
    /// PHY header, etc.
    type Headroom: ArrayLength<ArrayType<u8>: Copy>;

    /// HW-specific tailroom, e.g. space for hardware offloaded FCS calculation,
    /// RSSI, LQI, received length, etc.
    type Tailroom: ArrayLength<ArrayType<u8>: Copy>;

    /// The value of aMaxPhyPacketSize of the driver's PHY.
    type MaxPhyPacketSize: Unsigned;

    /// Drivers SHALL declare here whether they require a precalculated valid
    /// FCS on egress and/or whether they require the FCS to be validated by the
    /// framework on ingress.
    ///
    /// NOTE: Drivers that require buffer space for the FCS but do not require
    ///       the FCS to be calculated on egress and/or validated on ingress by
    ///       the framework SHALL set this to [`WithoutFcs`] and declare the
    ///       required space as [`RadioDriverConfig::Tailroom``] instead.
    type Fcs: StaticallySized + Copy + core::fmt::Debug;

    /// Required to calculate the correct buffer space for channel hopping IEs.
    // TODO: Derive from the max number of channels of the driver's channel page.
    type ExtBmLen: ArrayLength<ArrayType<u8>: Copy>;

    fn new() -> Self;
}

pub type Headroom<Config> = <Config as RadioDriverConfig>::Headroom;
pub type Tailroom<Config> = <Config as RadioDriverConfig>::Tailroom;
pub type MaxPhyPacketSize<Config> = <Config as RadioDriverConfig>::MaxPhyPacketSize;
pub type FcsType<Config> = <Config as RadioDriverConfig>::Fcs;
pub type FcsSize<Config> = SizeOf<<Config as RadioDriverConfig>::Fcs>;

#[derive(Clone, Copy)]
pub struct RadioDriverConfigBuilder<
    Headroom: ArrayLength<ArrayType<u8>: Copy>,
    Tailroom: ArrayLength<ArrayType<u8>: Copy>,
    MaxPhyPacketSize: Unsigned,
    Fcs: StaticallySized + Copy,
    ExtBmLen: ArrayLength<ArrayType<u8>: Copy>,
> {
    headroom: PhantomData<Headroom>,
    tailroom: PhantomData<Tailroom>,
    max_phy_packet_size: PhantomData<MaxPhyPacketSize>,
    fcs: PhantomData<Fcs>,
    ext_bm_len: PhantomData<ExtBmLen>,
}

impl<
        Headroom: ArrayLength<ArrayType<u8>: Copy>,
        Tailroom: ArrayLength<ArrayType<u8>: Copy>,
        MaxPhyPacketSize: Unsigned,
        Fcs: StaticallySized + Copy + core::fmt::Debug,
        ExtBmLen: ArrayLength<ArrayType<u8>: Copy>,
    > RadioDriverConfig
    for RadioDriverConfigBuilder<Headroom, Tailroom, MaxPhyPacketSize, Fcs, ExtBmLen>
{
    type Headroom = Headroom;
    type Tailroom = Tailroom;
    type MaxPhyPacketSize = MaxPhyPacketSize;
    type Fcs = Fcs;
    type ExtBmLen = ExtBmLen;

    fn new() -> Self {
        Self {
            headroom: PhantomData,
            tailroom: PhantomData,
            max_phy_packet_size: PhantomData,
            fcs: PhantomData,
            ext_bm_len: PhantomData,
        }
    }
}

#[allow(dead_code)]
pub fn radio_driver_config() -> RadioDriverConfigBuilder<U0, U0, MaxPhyPacketSize127, Fcs2Byte, U0>
{
    RadioDriverConfigBuilder::new()
}

#[allow(dead_code)]
impl<
        Headroom: ArrayLength<ArrayType<u8>: Copy>,
        Tailroom: ArrayLength<ArrayType<u8>: Copy>,
        MaxPhyPacketSize: Unsigned,
        Fcs: StaticallySized + Copy,
        ExtBmLen: ArrayLength<ArrayType<u8>: Copy>,
    > RadioDriverConfigBuilder<Headroom, Tailroom, MaxPhyPacketSize, Fcs, ExtBmLen>
{
    pub fn with_headroom<const N: usize>(
        self,
    ) -> RadioDriverConfigBuilder<U<N>, Tailroom, MaxPhyPacketSize, Fcs, ExtBmLen>
    where
        Const<N>: ToUInt,
        U<N>: ArrayLength,
        <U<N> as ArrayLength>::ArrayType<u8>: Copy,
    {
        RadioDriverConfigBuilder {
            headroom: PhantomData,
            tailroom: PhantomData,
            max_phy_packet_size: PhantomData,
            fcs: PhantomData,
            ext_bm_len: PhantomData,
        }
    }

    pub fn with_tailroom<const N: usize>(
        self,
    ) -> RadioDriverConfigBuilder<Headroom, U<N>, MaxPhyPacketSize, Fcs, ExtBmLen>
    where
        Const<N>: ToUInt,
        U<N>: ArrayLength,
        <U<N> as ArrayLength>::ArrayType<u8>: Copy,
    {
        RadioDriverConfigBuilder {
            headroom: PhantomData,
            tailroom: PhantomData,
            max_phy_packet_size: PhantomData,
            fcs: PhantomData,
            ext_bm_len: PhantomData,
        }
    }

    pub fn with_ext_bm_len<const N: usize>(
        self,
    ) -> RadioDriverConfigBuilder<Headroom, Tailroom, MaxPhyPacketSize, Fcs, U<N>>
    where
        Const<N>: ToUInt,
        U<N>: ArrayLength,
        <U<N> as ArrayLength>::ArrayType<u8>: Copy,
    {
        RadioDriverConfigBuilder {
            headroom: PhantomData,
            tailroom: PhantomData,
            max_phy_packet_size: PhantomData,
            fcs: PhantomData,
            ext_bm_len: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<
        Headroom: ArrayLength<ArrayType<u8>: Copy>,
        Tailroom: ArrayLength<ArrayType<u8>: Copy>,
        Fcs: StaticallySized + Copy,
        ExtBmLen: ArrayLength<ArrayType<u8>: Copy>,
    > RadioDriverConfigBuilder<Headroom, Tailroom, MaxPhyPacketSize127, Fcs, ExtBmLen>
{
    pub fn with_max_phy_packet_size_2047(
        self,
    ) -> RadioDriverConfigBuilder<Headroom, Tailroom, MaxPhyPacketSize2047, Fcs, ExtBmLen> {
        RadioDriverConfigBuilder {
            headroom: PhantomData,
            tailroom: PhantomData,
            max_phy_packet_size: PhantomData,
            fcs: PhantomData,
            ext_bm_len: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<
        Headroom: ArrayLength<ArrayType<u8>: Copy>,
        Tailroom: ArrayLength<ArrayType<u8>: Copy>,
        MaxPhyPacketSize: Unsigned,
        ExtBmLen: ArrayLength<ArrayType<u8>: Copy>,
    > RadioDriverConfigBuilder<Headroom, Tailroom, MaxPhyPacketSize, Fcs2Byte, ExtBmLen>
{
    pub fn without_fcs(
        self,
    ) -> RadioDriverConfigBuilder<Headroom, Tailroom, MaxPhyPacketSize2047, NoFcs, ExtBmLen> {
        RadioDriverConfigBuilder {
            headroom: PhantomData,
            tailroom: PhantomData,
            max_phy_packet_size: PhantomData,
            fcs: PhantomData,
            ext_bm_len: PhantomData,
        }
    }

    pub fn with_4byte_fcs(
        self,
    ) -> RadioDriverConfigBuilder<Headroom, Tailroom, MaxPhyPacketSize2047, Fcs4Byte, ExtBmLen>
    {
        RadioDriverConfigBuilder {
            headroom: PhantomData,
            tailroom: PhantomData,
            max_phy_packet_size: PhantomData,
            fcs: PhantomData,
            ext_bm_len: PhantomData,
        }
    }
}
