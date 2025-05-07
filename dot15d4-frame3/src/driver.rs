use generic_array::ArrayLength;
use typenum::Unsigned;

use crate::{Error, Frame, FrameToken, Result, TokenToFrame};
use core::{fmt::Debug, marker::PhantomData, ops::Range};

// PHY
#[allow(dead_code)]
pub const MAX_PHY_PACKET_SIZE_2047: usize = 2048; // SUN, TVWS, RCC, LECIM FSK, and MSK with a 2000 kb/s data rate
#[allow(dead_code)]
pub const MAX_PHY_PACKET_SIZE_127: usize = 127; // all other PHYs

// types allowed for DriverConfig::Fcs
// Drivers for LECIM, TVWS and SUN PHYs may be configured with a 4-byte FCS, all
// other drivers/PHYs use two bytes.
// Drivers that offload FCS (=CRC) checking to hardware will neither require
// nor include an FCS in the frame.
pub type FcsNone = ();
pub type FcsTwoBytes = u16;
pub type FcsFourBytes = u32;

pub trait DriverConfig {
    type Headroom: ArrayLength;
    type Tailroom: ArrayLength;
    // aMaxPhyPacketSize if the FCS is handled by the MAC, otherwise
    // aMaxPhyPacketSize minus FCS size.
    type MaxFrameLen: ArrayLength;
    type Fcs: Copy + Debug;
}

/// Provides a simple default driver frame representation implementation.
#[derive(Clone, Copy, Debug)]
pub struct DriverFrameRepr<Config: DriverConfig> {
    config: PhantomData<Config>,
}

impl<Config: DriverConfig> DriverFrameRepr<Config> {
    pub const fn new() -> Self {
        Self {
            config: PhantomData,
        }
    }

    pub const fn driver_frame_length(mpdu_length: usize) -> usize {
        mpdu_length + Self::driver_overhead()
    }

    pub const fn max_driver_frame_length() -> usize {
        <Config::MaxFrameLen as Unsigned>::USIZE
    }

    pub const fn driver_overhead() -> usize {
        <Config::Headroom as Unsigned>::USIZE + <Config::Tailroom as Unsigned>::USIZE
    }

    const fn sdu_range(buffer_len: usize) -> Range<usize> {
        <Config::Headroom as Unsigned>::USIZE..(buffer_len - <Config::Tailroom as Unsigned>::USIZE)
    }
}

pub struct Tx;
pub struct Rx;

/// Provides a simple default driver frame token implementation.
#[derive(Clone, Copy, Debug)]
pub struct DriverFrameToken<Config: DriverConfig, Direction> {
    config: PhantomData<Config>,
    length: usize,

    // Rx or Tx
    api: PhantomData<Direction>,
}

impl<Config: DriverConfig, Direction> DriverFrameToken<Config, Direction> {
    pub(crate) const fn new(sdu_length: usize) -> Self {
        let length = DriverFrameRepr::<Config>::driver_frame_length(sdu_length);

        // The default implementation assumes that no special condition must be
        // met for the driver to be able to receive/send frames.

        Self {
            config: PhantomData,
            length,
            api: PhantomData,
        }
    }
}

impl<Config: DriverConfig, Direction> FrameToken for DriverFrameToken<Config, Direction> {
    type Repr = DriverFrameRepr<Config>;

    async fn acquire(_repr: Self::Repr, sdu_length: usize) -> Result<Self> {
        Ok(Self::new(sdu_length))
    }
}

impl<'buffer, Config: DriverConfig, Direction> TokenToFrame<'buffer>
    for DriverFrameToken<Config, Direction>
{
    type Pdu = [u8];

    fn set_buffer(
        &mut self,
        buffer: &'buffer mut [u8],
    ) -> Result<impl Frame<'buffer, Pdu = Self::Pdu>> {
        if buffer.len() >= self.length {
            Ok(DriverFrame::<Config, Direction>::new(buffer))
        } else {
            Err(Error)
        }
    }
}

/// Provides a simple default driver frame implementation with an externally
/// allocated buffer.
#[derive(Debug)]
pub struct DriverFrame<'buffer, Config: DriverConfig, Direction> {
    config: PhantomData<Config>,

    /// The buffer allocated for the frame. None while no buffer has been
    /// allocated yet.
    ///
    /// Dropping the buffer SHALL release any resources held by the buffer even
    /// if the given buffer only represents a pointer to buffer space.
    ///
    /// NOTE: The buffer's capacity SHALL be greater or equal Repr::length() at
    ///       all times.
    buffer: &'buffer mut [u8],

    // Rx or Tx
    direction: PhantomData<Direction>,
}

impl<'buffer, Config: DriverConfig, Direction> DriverFrame<'buffer, Config, Direction> {
    pub const fn new(buffer: &'buffer mut [u8]) -> Self {
        Self {
            config: PhantomData,
            buffer,
            direction: PhantomData,
        }
    }
}

impl<'buffer, Config: DriverConfig, Direction> Frame<'buffer>
    for DriverFrame<'buffer, Config, Direction>
{
    type Pdu = [u8];

    fn buffer(self) -> &'buffer mut [u8] {
        self.buffer
    }

    fn pdu_ref(&self) -> &[u8] {
        &self.buffer
    }

    fn pdu_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    fn sdu_ref(&self) -> &[u8] {
        let buffer_len = self.buffer.len();
        let sdu = &self.buffer[DriverFrameRepr::<Config>::sdu_range(buffer_len)];
        sdu
    }

    fn sdu_mut(&mut self) -> &mut [u8] {
        let buffer_len = self.buffer.len();
        let sdu = &mut self.buffer[DriverFrameRepr::<Config>::sdu_range(buffer_len)];
        sdu
    }
}
