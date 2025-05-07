use core::{marker::PhantomData, ops::Add};

use bytemuck::AnyBitPattern;
use byteorder::{ByteOrder, LE};
use typenum::{Sum, Unsigned, U0, U2, U8};

use crate::{
    fc::{AddressingMode, FrameControl, FrameVersion},
    StaticallySized,
};

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Addressing<
    DstPanId: Copy,
    DstAddress: Copy,
    SrcPanId: Copy,
    SrcAddress: Copy,
    Size: Unsigned,
> {
    dst_pan_id: DstPanId,
    pub dst_addr: DstAddress,
    pub src_pan_id: SrcPanId,
    pub src_addr: SrcAddress,
    size: Size,
}

impl<DstPanId: Copy, DstAddress: Copy, SrcPanId: Copy, SrcAddress: Copy, Size: Unsigned>
    StaticallySized for Addressing<DstPanId, DstAddress, SrcPanId, SrcAddress, Size>
{
    type Size = Size;
}

#[allow(dead_code)]
impl<DstAddress: Copy, SrcPanId: Copy, SrcAddress: Copy, Size: Unsigned>
    Addressing<WithPanId, DstAddress, SrcPanId, SrcAddress, Size>
{
    pub fn set_dst_pan_id(&mut self, dst_pan_id: u16) {
        self.dst_pan_id = dst_pan_id.to_le();
    }

    pub fn get_dst_pan_id(&mut self) -> u16 {
        self.dst_pan_id.to_le()
    }
}

#[derive(Clone, Copy, AnyBitPattern)]
pub struct NoAddressing;
impl StaticallySized for NoAddressing {
    type Size = U0;
}

// types allowed for AddressingConfig::DstPanId and AddressingConfig::SrcPanId
#[allow(dead_code)]
pub type WithPanId = u16;
#[allow(dead_code)]
pub type WithoutPanId = ();

// types allowed for AddressingConfig::DstAddress and AddressingConfig::SrcAddress
#[allow(dead_code)]
pub type NoAddr = ();
#[allow(dead_code)]
pub type ShortAddr = u16;
#[allow(dead_code)]
pub type ExtendedAddr = [u8; 8];

#[derive(Clone, Copy)]
pub struct AddressingBuilder<
    DstPanId: Copy,
    DstAddress: Copy,
    SrcPanId: Copy,
    SrcAddress: Copy,
    Size: Unsigned,
> {
    addressing: PhantomData<Addressing<DstPanId, DstAddress, SrcPanId, SrcAddress, Size>>,
}

impl AddressingBuilder<WithoutPanId, NoAddr, WithoutPanId, NoAddr, U0> {
    fn new() -> Self {
        Self {
            addressing: PhantomData,
        }
    }
}

#[allow(dead_code)]
pub fn addressing() -> AddressingBuilder<WithoutPanId, NoAddr, WithoutPanId, NoAddr, U0> {
    AddressingBuilder::new()
}

#[allow(dead_code)]
impl<DstAddress: Copy, SrcPanId: Copy, SrcAddress: Copy, Size: Unsigned>
    AddressingBuilder<WithoutPanId, DstAddress, SrcPanId, SrcAddress, Size>
{
    pub fn with_dst_pan_id(
        self,
    ) -> AddressingBuilder<WithPanId, DstAddress, SrcPanId, SrcAddress, Sum<Size, U2>>
    where
        Size: Add<U2>,
        Sum<Size, U2>: Unsigned,
    {
        AddressingBuilder {
            addressing: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<DstPanId: Copy, SrcPanId: Copy, SrcAddress: Copy, Size: Unsigned>
    AddressingBuilder<DstPanId, NoAddr, SrcPanId, SrcAddress, Size>
{
    pub fn with_short_dst_addr(
        self,
    ) -> AddressingBuilder<WithPanId, ShortAddr, SrcPanId, SrcAddress, Sum<Size, U2>>
    where
        Size: Add<U2>,
        Sum<Size, U2>: Unsigned,
    {
        AddressingBuilder {
            addressing: PhantomData,
        }
    }

    pub fn with_extended_dst_addr(
        self,
    ) -> AddressingBuilder<WithPanId, ExtendedAddr, SrcPanId, SrcAddress, Sum<Size, U8>>
    where
        Size: Add<U8>,
        Sum<Size, U8>: Unsigned,
    {
        AddressingBuilder {
            addressing: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<DstPanId: Copy, DstAddress: Copy, SrcAddress: Copy, Size: Unsigned>
    AddressingBuilder<DstPanId, DstAddress, WithoutPanId, SrcAddress, Size>
{
    pub fn with_src_pan_id(
        self,
    ) -> AddressingBuilder<DstPanId, DstAddress, WithPanId, SrcAddress, Sum<Size, U2>>
    where
        Size: Add<U2>,
        Sum<Size, U2>: Unsigned,
    {
        AddressingBuilder {
            addressing: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<DstPanId: Copy, DstAddress: Copy, SrcPanId: Copy, Size: Unsigned>
    AddressingBuilder<DstPanId, DstAddress, SrcPanId, NoAddr, Size>
{
    pub fn with_short_src_addr(
        self,
    ) -> AddressingBuilder<WithPanId, DstAddress, SrcPanId, ShortAddr, Sum<Size, U2>>
    where
        Size: Add<U2>,
        Sum<Size, U2>: Unsigned,
    {
        AddressingBuilder {
            addressing: PhantomData,
        }
    }

    pub fn with_extended_src_addr(
        self,
    ) -> AddressingBuilder<WithPanId, DstAddress, SrcPanId, ExtendedAddr, Sum<Size, U8>>
    where
        Size: Add<U8>,
        Sum<Size, U8>: Unsigned,
    {
        AddressingBuilder {
            addressing: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<DstPanId: Copy, DstAddress: Copy, SrcPanId: Copy, SrcAddress: Copy, Size: Unsigned>
    AddressingBuilder<DstPanId, DstAddress, SrcPanId, SrcAddress, Size>
{
    pub fn finalize(
        self,
    ) -> PhantomData<Addressing<DstPanId, DstAddress, SrcPanId, SrcAddress, Size>> {
        PhantomData
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[allow(dead_code)]
pub enum Address<'frame> {
    Absent,
    Short(ShortAddr),
    Extended(&'frame [u8]),
} // 8 bytes

impl<'frame> Address<'frame> {
    /// The broadcast address.
    pub const BROADCAST: Self = Self::Short(0xffff);

    /// Query whether the address is an unicast address.
    pub fn is_unicast(&self) -> bool {
        !self.is_broadcast()
    }

    /// Query whether this address is the broadcast address.
    pub fn is_broadcast(&self) -> bool {
        *self == Self::BROADCAST
    }
}

pub struct ParsedAddress<'frame> {
    pub dst_pan_id: Option<u16>, // 2 bytes - will be derived if not set in the header
    pub dst_addr: Address<'frame>, // 8 bytes
    pub src_pan_id: Option<u16>, // 2 bytes - will be derived if not set in the header
    pub src_addr: Address<'frame>, // 8 bytes
}

pub struct ParsedAddressing<'frame> {
    content: &'frame [u8],
} // 8 bytes

impl<'frame> ParsedAddressing<'frame> {
    pub fn addresses(&self, fc: FrameControl) -> ParsedAddress {
        let PanConfig {
            dst_pan_present,
            src_pan_present,
        } = pan_compression(fc);

        let buffer = self.content;
        let (buffer, dst_pan_id, dst_addr) =
            Self::parse_pan_id_and_address(buffer, dst_pan_present, fc.dst_addr_mode());
        let (_, src_pan_id, src_addr) =
            Self::parse_pan_id_and_address(buffer, src_pan_present, fc.src_addr_mode());

        ParsedAddress {
            dst_pan_id,
            dst_addr,
            src_pan_id,
            src_addr,
        }
    }

    fn parse_pan_id_and_address(
        buffer: &[u8],
        pan_present: bool,
        addressing_mode: AddressingMode,
    ) -> (&[u8], Option<WithPanId>, Address) {
        let (buffer, pan_id) = match pan_present {
            true => (&buffer[2..], Some(LE::read_u16(buffer))),
            false => (buffer, None),
        };

        let (buffer, addr) = match addressing_mode {
            AddressingMode::None => (buffer, Address::Absent),
            AddressingMode::Short => (&buffer[2..], Address::Short(LE::read_u16(buffer))),
            AddressingMode::Extended => {
                let ext_addr = Address::Extended(&buffer[0..8]);
                (&buffer[8..], ext_addr)
            }
        };

        (buffer, pan_id, addr)
    }
}

struct PanConfig {
    dst_pan_present: bool,
    src_pan_present: bool,
}

pub fn parse_addressing(fc: FrameControl, buffer: &[u8]) -> Result<(&[u8], ParsedAddressing), ()> {
    let addressing_len = addressing_len(fc);
    let (buffer, content) = buffer.split_at(addressing_len);
    Ok((buffer, ParsedAddressing { content }))
}

fn pan_compression(fc: FrameControl) -> PanConfig {
    let (dst_pan_present, src_pan_present) = match fc.frame_version() {
        FrameVersion::Ieee802154_2003 | FrameVersion::Ieee802154_2006 => {
            (true, !fc.pan_id_compression())
        }
        FrameVersion::Ieee802154 => {
            /*
                Dst Addr    | Src Addr    | Dst Pan     | Src Pan     | PAN compr
                =================================================================
                Not present | Not present | Not present | Not present | 0
                Not present | Not present | Present     | Not present | 1
                Present     | Not present | Present     | Not present | 0
                Present     | Not present | Not present | Not present | 1
                Not present | Present     | Not present | Present     | 0
                Not present | Present     | Not present | Not present | 1

                Extended    | Extended    | Present     | Not present | 0
                Extended    | Extended    | Not present | Not present | 1

                Short*      | Short*      | Present     | Present     | 0
                Short*      | Extended    | Present     | Present     | 0
                Extended    | Short*      | Present     | Present     | 0

                Short*      | Extended    | Present     | Not present | 1
                Extended    | Short*      | Present     | Not present | 1
                Short*      | Short*      | Present     | Not present | 1

                * If both the destination and source addressing information is
                present and either is a short address, the MAC sublayer shall
                compare the destination and source PAN IDs and the PAN ID
                Compression field shall be set to zero if and only if the PAN
                identifiers are identical.
            */
            match (
                fc.dst_addr_mode(),
                fc.src_addr_mode(),
                fc.pan_id_compression(),
            ) {
                (AddressingMode::None, AddressingMode::None, false) => (false, false),
                (AddressingMode::None, AddressingMode::None, true) => (true, false),

                (AddressingMode::Short | AddressingMode::Extended, AddressingMode::None, false) => {
                    (true, false)
                }
                (AddressingMode::Short | AddressingMode::Extended, AddressingMode::None, true) => {
                    (false, false)
                }

                (AddressingMode::None, AddressingMode::Short | AddressingMode::Extended, false) => {
                    (false, true)
                }
                (AddressingMode::None, AddressingMode::Short | AddressingMode::Extended, true) => {
                    (false, false)
                }

                (AddressingMode::Extended, AddressingMode::Extended, false) => (true, false),
                (AddressingMode::Extended, AddressingMode::Extended, true) => (false, false),

                (AddressingMode::Short, AddressingMode::Short, false) => (true, true),
                (AddressingMode::Short, AddressingMode::Extended, false) => (true, true),
                (AddressingMode::Extended, AddressingMode::Short, false) => (true, true),

                (AddressingMode::Short, AddressingMode::Extended, true) => (true, false),
                (AddressingMode::Extended, AddressingMode::Short, true) => (true, false),
                (AddressingMode::Short, AddressingMode::Short, true) => (true, false),
            }
        }
        FrameVersion::Reserved => unreachable!(),
    };

    PanConfig {
        dst_pan_present,
        src_pan_present,
    }
}

fn addressing_len(fc: FrameControl) -> usize {
    let PanConfig {
        dst_pan_present,
        src_pan_present,
    } = pan_compression(fc);

    let mut len = 0;
    if dst_pan_present {
        len += 2;
    }
    if src_pan_present {
        len += 2;
    }
    len += match fc.dst_addr_mode() {
        AddressingMode::None => 0,
        AddressingMode::Short => 2,
        AddressingMode::Extended => 8,
    };
    len += match fc.src_addr_mode() {
        AddressingMode::None => 0,
        AddressingMode::Short => 2,
        AddressingMode::Extended => 8,
    };
    len
}
