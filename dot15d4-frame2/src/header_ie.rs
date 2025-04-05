use core::{marker::PhantomData, ops::Add};

use bitfield_struct::bitfield;
use bytemuck::AnyBitPattern;
use byteorder::{ByteOrder, LE};
use typenum::{Sum, Unsigned, U0, U2};

use crate::{frame::ParsedIe, SizeOf, StaticallySized};

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum HeaderIeId {
    TimeCorrectionIE = 0x1e,
    HeaderTerminationIE1 = 0x7e,
    HeaderTerminationIE2 = 0x7f,
}

impl HeaderIeId {
    const fn into_bits(self) -> u8 {
        self as _
    }
    const fn from_bits(value: u8) -> Self {
        match value {
            0x1e => Self::TimeCorrectionIE,
            0x7e => Self::HeaderTerminationIE1,
            0x7f => Self::HeaderTerminationIE2,
            _ => unreachable!(),
        }
    }
}

#[bitfield(u16)]
pub struct HeaderIeHdr {
    #[bits(7)]
    pub length: u16,
    #[bits(8)]
    pub element_id: HeaderIeId,
    #[bits(1)]
    pub ie_type: u8, // always zero
}

impl StaticallySized for HeaderIeHdr {
    type Size = U2;
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct HeaderIe<Content: Copy, Size: Unsigned> {
    pub hdr: HeaderIeHdr,
    pub content: Content,
    size: Size,
}

#[allow(dead_code)]
type HeaderIeSize<Content> = Sum<SizeOf<HeaderIeHdr>, SizeOf<Content>>;

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct HeaderIes<PrevHeaderIes: Copy, HeaderIe: Copy, Size: Unsigned> {
    pub prev: PrevHeaderIes,
    pub header_ie: HeaderIe,
    size: Size,
}

impl<PrevHeaderIes: Copy, HeaderIe: Copy, Size: Unsigned> StaticallySized
    for HeaderIes<PrevHeaderIes, HeaderIe, Size>
{
    type Size = Size;
}

#[derive(Clone, Copy)]
pub struct NoHeaderIes;
impl StaticallySized for NoHeaderIes {
    type Size = U0;
}

#[derive(Clone, Copy)]
pub struct HeaderIesBuilder<PrevHeaderIes: Copy, HeaderIe: Copy, Size: Unsigned> {
    header_ies: PhantomData<HeaderIes<PrevHeaderIes, HeaderIe, Size>>,
}

#[allow(dead_code)]
impl HeaderIesBuilder<(), (), U0> {
    /// Represents an empty header IE list.
    fn new() -> Self {
        Self {
            header_ies: PhantomData,
        }
    }
}

#[allow(dead_code)]
pub fn header_ies() -> HeaderIesBuilder<(), (), U0> {
    HeaderIesBuilder::new()
}

#[allow(dead_code)]
impl<ThisPrevHeaderIes: Copy, ThisHeaderIe: Copy, ThisSize: Unsigned>
    HeaderIesBuilder<ThisPrevHeaderIes, ThisHeaderIe, ThisSize>
{
    pub fn add_header_ie<Content: Copy, Size: Unsigned>(
        self,
        _header_ie: PhantomData<HeaderIe<Content, Size>>,
    ) -> HeaderIesBuilder<
        HeaderIes<ThisPrevHeaderIes, ThisHeaderIe, ThisSize>,
        HeaderIe<Content, Size>,
        Sum<ThisSize, Size>,
    >
    where
        ThisSize: Add<Size>,
        Sum<ThisSize, Size>: Unsigned,
    {
        HeaderIesBuilder {
            header_ies: PhantomData,
        }
    }

    pub fn finalize(self) -> PhantomData<HeaderIes<ThisPrevHeaderIes, ThisHeaderIe, ThisSize>> {
        PhantomData
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy, AnyBitPattern)]
pub struct TimeCorrectionIeContent {
    pub time_sync_info: u16,
}

impl StaticallySized for TimeCorrectionIeContent {
    type Size = U2;
}

#[allow(dead_code)]
pub type TimeCorrectionIeSize = HeaderIeSize<TimeCorrectionIeContent>;

#[allow(dead_code)]
pub fn time_correction_ie() -> PhantomData<HeaderIe<TimeCorrectionIeContent, TimeCorrectionIeSize>>
{
    PhantomData
}

#[derive(Clone, Copy)]
pub struct HeaderTerminationIe1Content;

impl StaticallySized for HeaderTerminationIe1Content {
    type Size = U0;
}

#[allow(dead_code)]
type HeaderTerminationIe1Size = HeaderIeSize<HeaderTerminationIe1Content>;

#[allow(dead_code)]
pub fn header_termination_ie_1(
) -> PhantomData<HeaderIe<HeaderTerminationIe1Content, HeaderTerminationIe1Size>> {
    PhantomData
}

#[derive(Clone, Copy)]
pub struct HeaderTerminationIe2Content;

impl StaticallySized for HeaderTerminationIe2Content {
    type Size = U0;
}

#[allow(dead_code)]
type HeaderTerminationIe2Size = HeaderIeSize<HeaderTerminationIe2Content>;

#[allow(dead_code)]
pub fn header_termination_ie_2(
) -> PhantomData<HeaderIe<HeaderTerminationIe2Content, HeaderTerminationIe2Size>> {
    PhantomData
}

pub fn parse_header_ie(buffer: &[u8]) -> Result<(&[u8], ParsedIe), ()> {
    let ie_hdr = HeaderIeHdr::from_bits(LE::read_u16(buffer));
    let buffer = &buffer[2..];

    if ie_hdr.ie_type() != 0 {
        return Err(());
    }

    let (buffer, parsed_ie) = match ie_hdr.element_id() {
        HeaderIeId::TimeCorrectionIE => parse_time_correction_ie(ie_hdr, buffer)?,
        HeaderIeId::HeaderTerminationIE1 => (buffer, ParsedIe::HeaderTerminationIe1),
        HeaderIeId::HeaderTerminationIE2 => (buffer, ParsedIe::HeaderTerminationIe2),
    };

    Ok((buffer, parsed_ie))
}

fn parse_time_correction_ie(ie_hdr: HeaderIeHdr, buffer: &[u8]) -> Result<(&[u8], ParsedIe), ()> {
    if ie_hdr.length() != 2 {
        return Err(());
    }
    let time_correction_ie = bytemuck::from_bytes(&buffer[0..2]);
    Ok((&buffer[2..], ParsedIe::TimeCorrectionIe(time_correction_ie)))
}
