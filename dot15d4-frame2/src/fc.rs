use bitfield_struct::bitfield;
use bytemuck::AnyBitPattern;
use typenum::U2;

use crate::StaticallySized;

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum FrameType {
    Beacon = 0b000,
    Data = 0b001,
    Acknowledgment = 0b010,
    MacCommand = 0b011,
    Reserved = 0b100,
    Multipurpose = 0b101,
    FragmentOrFrak = 0b110,
    Extended = 0b111,
}

impl FrameType {
    // This has to be a const fn
    const fn into_bits(self) -> u8 {
        self as _
    }
    const fn from_bits(value: u8) -> Self {
        match value {
            0b000 => Self::Beacon,
            0b001 => Self::Data,
            0b010 => Self::Acknowledgment,
            0b011 => Self::MacCommand,
            _ => unreachable!(),
        }
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum AddressingMode {
    None = 0b00,
    Short = 0b10,
    Extended = 0b11,
}

impl AddressingMode {
    // This has to be a const fn
    const fn into_bits(self) -> u8 {
        self as _
    }
    const fn from_bits(value: u8) -> Self {
        match value {
            0b00 => Self::None,
            0b10 => Self::Short,
            0b11 => Self::Extended,
            _ => unreachable!(),
        }
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum PanIdCompression {
    No = 0x0,
    Yes = 0x1,
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum SeqNumSuppression {
    No = 0x0,
    Yes = 0x1,
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum FrameVersion {
    Ieee802154_2003 = 0b00,
    Ieee802154_2006 = 0b01,
    Ieee802154 = 0b10,
    Reserved = 0b11,
}

impl FrameVersion {
    // This has to be a const fn
    const fn into_bits(self) -> u8 {
        self as _
    }
    const fn from_bits(value: u8) -> Self {
        match value {
            0b00 => Self::Ieee802154_2003,
            0b01 => Self::Ieee802154_2006,
            0b10 => Self::Ieee802154,
            _ => unreachable!(),
        }
    }
}

#[bitfield(u16)]
#[derive(AnyBitPattern)]
pub struct FrameControl {
    #[bits(3)]
    pub frame_type: FrameType,
    #[bits(1)]
    pub security_enabled: bool,
    #[bits(1)]
    pub frame_pending: bool,
    #[bits(1)]
    pub ack_requested: bool,
    #[bits(1)]
    pub pan_id_compression: bool,
    #[bits(1)]
    pub reserved: bool,
    #[bits(1)]
    pub seq_num_suppr: bool,
    #[bits(1)]
    pub ie_present: bool,
    #[bits(2)]
    pub dst_addr_mode: AddressingMode,
    #[bits(2)]
    pub frame_version: FrameVersion,
    #[bits(2)]
    pub src_addr_mode: AddressingMode,
}

impl StaticallySized for FrameControl {
    type Size = U2;
}

#[allow(dead_code)]
pub fn fc_ieee802154_2003(frame_type: FrameType) -> FrameControl {
    FrameControl::new()
        .with_frame_type(frame_type)
        .with_frame_version(FrameVersion::Ieee802154_2003)
}

#[allow(dead_code)]
pub fn fc_ieee802154_2006(frame_type: FrameType) -> FrameControl {
    FrameControl::new()
        .with_frame_type(frame_type)
        .with_frame_version(FrameVersion::Ieee802154_2006)
}

#[allow(dead_code)]
pub fn fc_ieee802154(frame_type: FrameType) -> FrameControl {
    FrameControl::new()
        .with_frame_type(frame_type)
        .with_frame_version(FrameVersion::Ieee802154)
}
