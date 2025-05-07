use core::{marker::PhantomData, ops::Add};

use bitfield_struct::bitfield;
#[cfg(feature = "with_security")]
use byteorder::{ByteOrder, LE};
use typenum::{Sum, Unsigned, U0, U1, U4, U8};

use crate::{addr::Address, command::CommandId, fc::FrameType, StaticallySized};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityLevel {
    None = 0b000,
    Mic32 = 0b001,
    Mic64 = 0b010,
    Mic128 = 0b011,
    EncMic32 = 0b101,
    EncMic64 = 0b110,
    EncMic128 = 0b111,
}

impl SecurityLevel {
    // This has to be a const fn
    const fn into_bits(self) -> u8 {
        self as _
    }
    const fn from_bits(value: u8) -> Self {
        match value {
            0b000 => Self::None,
            0b001 => Self::Mic32,
            0b010 => Self::Mic64,
            0b011 => Self::Mic128,
            0b101 => Self::EncMic32,
            0b110 => Self::EncMic64,
            0b111 => Self::EncMic128,
            _ => unreachable!(),
        }
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum KeyIdMode {
    Implicit = 0b00,
    SourceNone = 0b01,
    Source4Byte = 0b10,
    Source8Byte = 0b11,
}

impl KeyIdMode {
    // This has to be a const fn
    const fn into_bits(self) -> u8 {
        self as _
    }
    const fn from_bits(value: u8) -> Self {
        match value {
            0b00 => Self::Implicit,
            0b01 => Self::SourceNone,
            0b10 => Self::Source4Byte,
            0b11 => Self::Source8Byte,
            _ => unreachable!(),
        }
    }
}

#[bitfield(u8)]
pub struct SecurityControl {
    #[bits(3)]
    pub sec_level: SecurityLevel,
    #[bits(2)]
    pub key_id_mode: KeyIdMode,
    #[bits(1)]
    pub frame_cnt_supp: bool,
    #[bits(1)]
    pub asn_in_nonce: bool,
    #[bits(1)]
    pub reserved: bool,
}
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct KeyId<KeySource> {
    pub key_src: KeySource,
    pub key_idx: u8,
}

// types allowed for SecurityConfig::FrameCounter
#[allow(dead_code)]
pub type WithFrameCounter = u32;
#[allow(dead_code)]
pub type WithoutFrameCounter = ();

// types allowed for SecurityConfig::KeyIdentifier
#[allow(dead_code)]
pub type KeyIdImplicit = ();
#[allow(dead_code)]
pub type KeyIdSourceNone = KeyId<()>; // key index
#[allow(dead_code)]
pub type KeyIdSource4Byte = KeyId<u32>; // 4-byte key source + key index
#[allow(dead_code)]
pub type KeyIdSource8Byte = KeyId<u64>; // 8-byte key source + key index

#[allow(dead_code)]
pub enum KeyIdVariant {
    Implicit,
    SourceNone(KeyIdSourceNone),
    Source4Byte(KeyIdSource4Byte),
    Source8Byte(KeyIdSource8Byte),
} // 10 bytes

#[allow(dead_code)]
pub struct SecurityContext {
    pub security_level: SecurityLevel,
    pub key_id: KeyIdVariant,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct AuxSecHeader<FrameCounter: Copy, KeyId: Copy, Size: Unsigned> {
    pub sec_ctrl: SecurityControl,
    pub frame_counter: FrameCounter,
    pub key_id: KeyId,
    size: Size,
}

impl<FrameCounter: Copy, KeyId: Copy, Size: Unsigned> StaticallySized
    for AuxSecHeader<FrameCounter, KeyId, Size>
{
    type Size = Size;
}

#[derive(Clone, Copy)]
pub struct NoAuxSecHeader;
impl StaticallySized for NoAuxSecHeader {
    type Size = U0;
}

#[derive(Clone, Copy)]
pub struct AuxSecHeaderBuilder<FrameCounter: Copy, KeyId: Copy, Size: Unsigned> {
    aux_sec_hdr: PhantomData<AuxSecHeader<FrameCounter, KeyId, Size>>,
}

impl AuxSecHeaderBuilder<WithoutFrameCounter, KeyIdImplicit, U1> {
    fn new() -> Self {
        Self {
            aux_sec_hdr: PhantomData,
        }
    }
}

#[allow(dead_code)]
pub fn aux_sec_hdr() -> AuxSecHeaderBuilder<WithoutFrameCounter, KeyIdImplicit, U1> {
    AuxSecHeaderBuilder::new()
}

#[allow(dead_code)]
impl<KeyId: Copy, Size: Unsigned> AuxSecHeaderBuilder<WithoutFrameCounter, KeyId, Size> {
    pub fn with_frame_counter(self) -> AuxSecHeaderBuilder<WithFrameCounter, KeyId, Sum<Size, U4>>
    where
        Size: Add<U4>,
        Sum<Size, U4>: Unsigned,
    {
        AuxSecHeaderBuilder {
            aux_sec_hdr: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<FrameCounter: Copy, Size: Unsigned> AuxSecHeaderBuilder<FrameCounter, KeyIdImplicit, Size> {
    pub fn with_key_id(self) -> AuxSecHeaderBuilder<FrameCounter, KeyIdSourceNone, Sum<Size, U1>>
    where
        Size: Add<U1>,
        Sum<Size, U1>: Unsigned,
    {
        AuxSecHeaderBuilder {
            aux_sec_hdr: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<FrameCounter: Copy, Size: Unsigned> AuxSecHeaderBuilder<FrameCounter, KeyIdSourceNone, Size> {
    pub fn with_4byte_source(
        self,
    ) -> AuxSecHeaderBuilder<FrameCounter, KeyIdSource4Byte, Sum<Size, U4>>
    where
        Size: Add<U4>,
        Sum<Size, U4>: Unsigned,
    {
        AuxSecHeaderBuilder {
            aux_sec_hdr: PhantomData,
        }
    }

    pub fn with_8byte_source(
        self,
    ) -> AuxSecHeaderBuilder<FrameCounter, KeyIdSource8Byte, Sum<Size, U8>>
    where
        Size: Add<U8>,
        Sum<Size, U8>: Unsigned,
    {
        AuxSecHeaderBuilder {
            aux_sec_hdr: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<FrameCounter: Copy, KeyId: Copy, Size: Unsigned>
    AuxSecHeaderBuilder<FrameCounter, KeyId, Size>
{
    pub fn finalize(self) -> PhantomData<AuxSecHeader<WithFrameCounter, KeyId, Size>> {
        PhantomData
    }
}

#[allow(dead_code)]
#[cfg(feature = "with_security")]
pub struct ParsedAuxSecHeader {
    pub sc: SecurityControl,                     // 1 byte
    pub frame_counter: Option<WithFrameCounter>, // 8 bytes
    pub key_id: KeyIdVariant,                    // 10 bytes
} // 20 bytes

#[allow(dead_code)]
#[cfg(feature = "with_security")]
pub fn parse_aux_sec_header(buffer: &[u8]) -> Result<(&[u8], ParsedAuxSecHeader), ()> {
    let sc = SecurityControl::from_bits(buffer[0]);
    let buffer = &buffer[1..];

    let frame_counter = match sc.frame_cnt_supp() {
        true => None,
        false => Some(LE::read_u32(buffer)),
    };
    let buffer = &buffer[4..];

    let (buffer, key_id) = match sc.key_id_mode() {
        KeyIdMode::Implicit => (buffer, KeyIdVariant::Implicit),
        KeyIdMode::SourceNone => (
            &buffer[1..],
            KeyIdVariant::SourceNone(KeyId {
                key_src: (),
                key_idx: buffer[0],
            }),
        ),
        KeyIdMode::Source4Byte => (
            &buffer[5..],
            KeyIdVariant::Source4Byte(KeyId {
                key_src: LE::read_u32(&buffer[1..5]),
                key_idx: buffer[0],
            }),
        ),
        KeyIdMode::Source8Byte => (
            &buffer[9..],
            KeyIdVariant::Source8Byte(KeyId {
                key_src: LE::read_u64(&buffer[1..9]),
                key_idx: buffer[0],
            }),
        ),
    };

    Ok((
        buffer,
        ParsedAuxSecHeader {
            sc,
            frame_counter,
            key_id,
        },
    ))
}

#[allow(dead_code)]
pub struct SecKeyDescriptor<'skd> {
    pub sec_key_usage_list: &'skd [SecKeyUsageDescriptor],
    pub sec_key: [u8; 16],
    pub sec_key_frame_counter: u32,
    pub sec_frame_counter_per_key: bool,
    pub sec_key_device_frame_counter_list: &'skd [SecKeyDeviceFrameCounter],
}

#[allow(dead_code)]
pub struct SecKeyDeviceFrameCounter {
    pub sec_device_ext_address: [u8; 8],
    pub sec_device_frame_counter: u32,
}

#[allow(dead_code)]
pub struct SecKeyUsageDescriptor {
    pub sec_key_usage_frame_type: FrameType,
    pub sec_key_usage_command_id: CommandId,
}

/// See IEEE 802.15.4-2020, section 9.2.3.
///
/// KeyIdMode mapping:
/// - implicit: key_index None
/// - no source: key_index Some, key_source None
/// - source 4/8 bytes: key_index Some, key_source Some
pub fn lookup_key_descriptor<'skd>(
    _key_id: KeyIdVariant,
    _device_pan_id: Option<u16>,
    _device_address: Address,
) -> Option<SecKeyDescriptor<'skd>> {
    todo!()
}
