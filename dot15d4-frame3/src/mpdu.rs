use core::marker::PhantomData;

use typenum::Unsigned;

#[cfg(feature = "tsch")]
use crate::ie::IeList;
#[cfg(feature = "security")]
use crate::sec::Security;
use crate::{
    driver::{DriverConfig, DriverFrame, DriverFrameRepr, DriverFrameToken, Rx, Tx},
    frame_control::{FrameControl, FrameType, FrameVersion},
    ie::MacIeConfig,
    sec::MacSecurityConfig,
    Address, Addressing, AddressingMode, Frame, FrameToken, Result, TokenToFrame,
};

#[derive(Clone, Copy, Debug)]
pub enum SeqNr {
    Yes,
    No,
} // 1 byte

impl SeqNr {
    pub const fn length(&self) -> usize {
        match self {
            SeqNr::Yes => 1,
            SeqNr::No => 0,
        }
    }
}

/// The frame builder represents minimal structural information about a frame in
/// as much a compressed form as possible for best runtime efficiency.
///
/// Content will be added on-the-fly by a writer-style implementation.
#[derive(Clone, Copy, Debug)]
pub struct MpduBuilder<'ies, Stage = Initial> {
    pub(crate) tsch_mode: bool,
    pub(crate) seq_nr: SeqNr,
    pub(crate) addressing: Option<Addressing>,
    #[cfg(feature = "security")]
    pub(crate) security: Option<Security>,
    #[cfg(feature = "tsch")]
    pub(crate) ies: Option<IeList<'ies>>,
    pub(crate) stage: PhantomData<&'ies Stage>, // Lifetime reference required in case TSCH is disabled.
}

pub type MpduRepr<'ies> = MpduBuilder<'ies, MacFrameConfigComplete>;

#[derive(Clone, Copy, Debug)]
pub struct Initial;

#[derive(Clone, Copy, Debug)]
pub struct MacFrameConfig;

#[derive(Clone, Copy, Debug)]
pub struct MacFrameConfigComplete;

// If a single driver implementation is linked to the crate then we can access
// its configuration at compile time.

impl<'frame> MpduBuilder<'frame, Initial> {
    pub const fn new() -> MpduBuilder<'frame, MacSecurityConfig> {
        MpduBuilder {
            tsch_mode: false,
            seq_nr: SeqNr::No,
            addressing: None,
            #[cfg(feature = "security")]
            security: None,
            #[cfg(feature = "tsch")]
            ies: None,
            stage: PhantomData,
        }
    }
}

impl<'frame> MpduBuilder<'frame, MacFrameConfig> {
    pub const fn with_frame_config(
        self,
        tsch_mode: bool,
        seq_nr: SeqNr,
        addressing: Addressing,
    ) -> MpduBuilder<'frame, MacIeConfig> {
        MpduBuilder {
            tsch_mode,
            seq_nr,
            addressing: Some(addressing),
            #[cfg(feature = "security")]
            security: self.security,
            #[cfg(feature = "tsch")]
            ies: None,
            stage: PhantomData,
        }
    }
}

impl<'frame> MpduBuilder<'frame, MacFrameConfigComplete> {
    const fn length<Config: DriverConfig>(&self, frame_payload_length: usize) -> usize {
        let mpdu_length = self.mpdu_length(frame_payload_length);
        DriverFrameRepr::<Config>::driver_frame_length(mpdu_length)
    }

    const fn mpdu_length(&self, frame_payload_length: usize) -> usize {
        const FC_LEN: usize = 2;

        let mut len = FC_LEN + self.seq_nr.length();

        len += match &self.addressing {
            Some(addressing) => addressing.length(),
            None => 0,
        };

        #[cfg(feature = "security")]
        {
            len += match &self.security {
                Some(security) => security.length(self.tsch_mode),
                None => 0,
            };
        }

        #[cfg(feature = "tsch")]
        {
            len += match &self.ies {
                Some(ie_list) => ie_list.length(frame_payload_length > 0),
                None => 0,
            };
        }

        len + frame_payload_length
    }

    pub fn new_tx_frame<Config: DriverConfig>(
        self,
        frame_version: FrameVersion,
        frame_type: FrameType,
        buffer: &'frame mut [u8],
    ) -> Result<MpduFrame<'frame, Tx>> {
        let mut mpdu = MpduFrame::<Tx>::from_buffer(
            self,
            buffer,
            <Config::Headroom as Unsigned>::U16,
            <Config::Tailroom as Unsigned>::U16,
        );

        let seq_num_suppression = matches!(mpdu.repr.seq_nr, SeqNr::No);
        let (dst_addr_mode, src_addr_mode, pan_id_compression) = match &mpdu.repr.addressing {
            Some(addressing) => (
                addressing.dst_addr_mode(),
                addressing.src_addr_mode(),
                addressing.pan_id_compression(),
            ),
            None => (AddressingMode::Absent, AddressingMode::Absent, false),
        };

        #[cfg(feature = "security")]
        let security_enabled = mpdu.repr.security.is_some();

        #[cfg(feature = "tsch")]
        let ie_present = mpdu.repr.ies.is_some();

        let mut fc = mpdu.frame_control_mut();
        fc.set_frame_version(frame_version);
        fc.set_frame_type(frame_type);

        if seq_num_suppression {
            fc.set_sequence_number_suppression(true);
        }

        if pan_id_compression {
            fc.set_pan_id_compression(pan_id_compression);
        }

        if !matches!(dst_addr_mode, AddressingMode::Absent) {
            fc.set_dst_addressing_mode(dst_addr_mode);
        }

        if !matches!(src_addr_mode, AddressingMode::Absent) {
            fc.set_src_addressing_mode(src_addr_mode);
        }

        #[cfg(feature = "security")]
        if security_enabled {
            fc.set_security_enabled(true);
        }

        #[cfg(feature = "tsch")]
        if ie_present {
            fc.set_information_elements_present(true);
        }

        Ok(mpdu)
    }
}

pub const fn frame_repr() -> MpduBuilder<'static, MacSecurityConfig> {
    MpduBuilder::new()
}

/// Represents frame allocation from the perspective of a given network layer.
#[derive(Clone, Copy, Debug)]
pub struct MpduToken<'frame, Config: DriverConfig, Direction> {
    /// Representation of the frame as seen from the current network layer.
    repr: MpduRepr<'frame>,

    driver_token: DriverFrameToken<Config, Direction>,
}

impl<'frame, Config: DriverConfig, Direction> MpduToken<'frame, Config, Direction> {
    const fn new(repr: MpduRepr<'frame>, frame_payload_length: usize) -> Self {
        let mpdu_length = repr.mpdu_length(frame_payload_length);
        let driver_token = DriverFrameToken::<Config, Direction>::new(mpdu_length);
        Self { repr, driver_token }
    }
}

impl<'frame, Config: DriverConfig, Direction> FrameToken for MpduToken<'frame, Config, Direction> {
    type Repr = MpduRepr<'frame>;

    async fn acquire(repr: MpduRepr<'frame>, sdu_length: usize) -> Result<Self> {
        Ok(Self::new(repr, sdu_length))
    }
}

impl<'frame, Config: DriverConfig + 'frame, Direction: 'frame> TokenToFrame<'frame>
    for MpduToken<'frame, Config, Direction>
{
    type Pdu = MpduFrame<'frame, Direction>;

    fn set_buffer(
        &mut self,
        buffer: &'frame mut [u8],
    ) -> Result<impl Frame<'frame, Pdu = Self::Pdu>> {
        debug_assert!(
            buffer.len() >= DriverFrameRepr::<Config>::driver_overhead() + self.repr.mpdu_length(0)
        );
        Ok(MpduFrame::from_buffer(
            self.repr,
            buffer,
            <Config::Headroom as Unsigned>::U16,
            <Config::Tailroom as Unsigned>::U16,
        ))
    }
}

/// Represents frame allocation from the perspective of a given network layer.
pub struct MpduFrame<'mpdu, Direction> {
    /// Representation of the frame as seen from the current network layer.
    repr: MpduRepr<'mpdu>,
    buffer: &'mpdu mut [u8],
    headroom: u16,
    tailroom: u16,
    direction: PhantomData<Direction>,
}

impl<'mpdu, Direction> Frame<'mpdu> for MpduFrame<'mpdu, Direction> {
    type Pdu = Self;

    fn buffer(self) -> &'mpdu mut [u8] {
        self.buffer
    }

    fn pdu_ref(&self) -> &Self {
        self
    }

    fn pdu_mut(&mut self) -> &mut Self {
        self
    }

    fn sdu_ref(&self) -> &[u8] {
        todo!()
    }

    fn sdu_mut(&mut self) -> &mut [u8] {
        todo!()
    }
}

impl<'buffer> MpduFrame<'buffer, Rx> {
    pub fn parse<Config: DriverConfig + 'buffer>(
        driver_frame: DriverFrame<'buffer, Config, Rx>,
    ) -> MpduFrame<'buffer, Rx> {
        // TODO: Lazily parse the frame on-demand to establish its actual
        //       representation! Don't parse everything right away as we often
        //       only need access to a few initial field, especially on
        //       time-sensitive paths (eg. packet filtering, ACK, ...).
        let repr = MpduBuilder::new()
            .without_security()
            .with_frame_config(
                false,
                SeqNr::Yes,
                Addressing {
                    dst: Address::Short(0x0000),
                    src: Address::Short(0x0000),
                    pan: crate::PanIdCompression::Legacy,
                },
            )
            .without_ies();
        MpduFrame {
            repr,
            buffer: driver_frame.buffer(),
            headroom: <Config::Headroom as Unsigned>::U16,
            tailroom: <Config::Tailroom as Unsigned>::U16,
            direction: PhantomData,
        }
    }
}

impl<'buffer, Direction> MpduFrame<'buffer, Direction> {
    const fn from_buffer(
        repr: MpduRepr<'buffer>,
        buffer: &'buffer mut [u8],
        headroom: u16,
        tailroom: u16,
    ) -> Self {
        debug_assert!(buffer.len() >= headroom as usize + repr.mpdu_length(0));
        Self {
            repr,
            buffer,
            headroom,
            tailroom,
            direction: PhantomData,
        }
    }

    pub fn driver_frame<Config: DriverConfig>(self) -> DriverFrame<'buffer, Config, Direction> {
        debug_assert_eq!(self.headroom, <Config::Headroom as Unsigned>::U16);
        debug_assert_eq!(self.tailroom, <Config::Tailroom as Unsigned>::U16);
        DriverFrame::<Config, Direction>::new(self.buffer)
    }

    fn mpdu_ref(&self) -> &[u8] {
        &self.buffer[self.headroom as _..(self.buffer.len() - self.tailroom as usize)]
    }

    fn mpdu_mut(&mut self) -> &mut [u8] {
        let buf_len = self.buffer.len();
        &mut self.buffer[self.headroom as _..(buf_len - self.tailroom as usize)]
    }

    /// Provides access to the [`FrameControl`] field.
    pub fn frame_control(&self) -> FrameControl<&[u8]> {
        let bytes = &self.mpdu_ref()[..2];
        FrameControl::new_unchecked(bytes)
    }

    /// Provides access to the [`FrameControl`] field.
    pub fn frame_control_mut(&mut self) -> FrameControl<&mut [u8]> {
        let bytes = &mut self.mpdu_mut()[..2];
        FrameControl::new_unchecked(bytes)
    }

    /// Reads the sequence number field.
    pub fn sequence_number(&self) -> Option<u8> {
        if !matches!(self.repr.seq_nr, SeqNr::Yes) {
            return None;
        }
        Some(self.mpdu_ref()[2])
    }

    /// Writes the sequence number field.
    pub fn with_sequence_number(mut self, seq_num: u8) -> Self {
        debug_assert!(matches!(self.repr.seq_nr, SeqNr::Yes));
        self.mpdu_mut()[2] = seq_num;
        self
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buffer
    }
}

pub const IMM_ACK_FRAME_REPR: MpduBuilder<MacFrameConfigComplete> = frame_repr()
    .without_security()
    .with_frame_config(false, SeqNr::Yes, Addressing::none())
    .without_ies();
pub const IMM_ACK_BUF_LEN: usize = IMM_ACK_FRAME_REPR.length(0) as usize;

/// Allocates an ImmAck frame on the stack and initializes it.
pub fn imm_ack_frame<Config: DriverConfig>(seq_num: u8, buffer: &mut [u8]) -> MpduFrame<Tx> {
    IMM_ACK_FRAME_REPR
        .new_tx_frame::<Config>(FrameVersion::Ieee802154_2006, FrameType::Ack, buffer)
        .unwrap()
        .with_sequence_number(seq_num)
}
