use core::{
    marker::PhantomData,
    ops::{Add, Mul},
};

use crate::{frame::ParsedIe, SizeOf, StaticallySized};
use bitfield_struct::bitfield;
use bytemuck::AnyBitPattern;
use byteorder::{ByteOrder, LE};
use generic_array::{ArrayLength, GenericArray};
use typenum::{Const, Prod, Sum, ToUInt, Unsigned, U, U0, U1, U12, U2, U27, U4, U5, U6};

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum PayloadIeGroupId {
    MlmeIe = 0x1,
    PayloadTerminationIe = 0xf,
}

impl PayloadIeGroupId {
    const fn into_bits(self) -> u8 {
        self as _
    }
    const fn from_bits(value: u8) -> Self {
        match value {
            0x1 => Self::MlmeIe,
            0xf => Self::PayloadTerminationIe,
            _ => unreachable!(),
        }
    }
}

#[bitfield(u16)]
pub struct PayloadIeHdr {
    #[bits(11)]
    pub length: u16,
    #[bits(4)]
    pub group_id: PayloadIeGroupId,
    #[bits(1)]
    pub ie_type: u8, // always one
}

pub type PayloadIeHdrSize = U2;

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct PayloadIe<Content: Copy, Size: Unsigned> {
    pub hdr: PayloadIeHdr,
    pub content: Content,
    size: PhantomData<Size>,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct PayloadIes<PrevPayloadIes: Copy, PayloadIe: Copy, Size: Unsigned> {
    pub prev: PrevPayloadIes,
    pub payload_ie: PayloadIe,
    size: PhantomData<Size>,
}

impl<PrevPayloadIes: Copy, PayloadIe: Copy, Size: Unsigned> StaticallySized
    for PayloadIes<PayloadIe, PrevPayloadIes, Size>
{
    type Size = Size;
}

#[derive(Clone, Copy)]
pub struct NoPayloadIes;

impl StaticallySized for NoPayloadIes {
    type Size = U0;
}

#[derive(Clone, Copy)]
pub struct PayloadIesBuilder<PayloadIes: Copy> {
    payload_ies: PhantomData<PayloadIes>,
}

#[allow(dead_code)]
impl PayloadIesBuilder<()> {
    /// Represents an empty header IE list.
    fn new() -> Self {
        Self {
            payload_ies: PhantomData,
        }
    }

    pub fn add_payload_ie<Content: Copy, Size: Unsigned>(
        self,
        _payload_ie: PhantomData<PayloadIe<Content, Size>>,
    ) -> PayloadIesBuilder<PayloadIes<(), PayloadIe<Content, Size>, Size>> {
        PayloadIesBuilder {
            payload_ies: PhantomData,
        }
    }

    pub fn finalize(self) -> PhantomData<PayloadIes<(), (), U0>> {
        PhantomData
    }
}

#[allow(dead_code)]
pub fn payload_ies() -> PayloadIesBuilder<()> {
    PayloadIesBuilder::new()
}

#[allow(dead_code)]
impl<ThisPrevPayloadIes: Copy, ThisPayloadIe: Copy, ThisSize: Unsigned>
    PayloadIesBuilder<PayloadIes<ThisPrevPayloadIes, ThisPayloadIe, ThisSize>>
{
    pub fn add_payload_ie<Content: Copy, Size: Unsigned>(
        self,
        _payload_ie: PhantomData<PayloadIe<Content, Size>>,
    ) -> PayloadIesBuilder<
        PayloadIes<
            PayloadIes<ThisPrevPayloadIes, ThisPayloadIe, ThisSize>,
            PayloadIe<Content, Size>,
            Sum<ThisSize, Size>,
        >,
    >
    where
        ThisSize: Add<Size>,
        Sum<ThisSize, Size>: Unsigned,
    {
        PayloadIesBuilder {
            payload_ies: PhantomData,
        }
    }

    pub fn finalize(self) -> PhantomData<PayloadIes<ThisPrevPayloadIes, ThisPayloadIe, ThisSize>> {
        PhantomData
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct MlmeIe<PrevNestedIes: Copy, NestedIe: Copy, Size: Unsigned> {
    pub prev: PrevNestedIes,
    pub nested_ie: NestedIe,
    size: PhantomData<Size>,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct MlmeIeBuilder<PrevNestedIes: Copy, NestedIe: Copy, Size: Unsigned> {
    pub mlme_ie: PhantomData<MlmeIe<PrevNestedIes, NestedIe, Size>>,
}

impl MlmeIeBuilder<(), (), U0> {
    fn new() -> Self {
        Self {
            mlme_ie: PhantomData,
        }
    }
}

#[allow(dead_code)]
pub fn mlme_ie() -> MlmeIeBuilder<(), (), U0> {
    MlmeIeBuilder::new()
}

#[allow(dead_code)]
impl<ThisPrevNestedIes: Copy, ThisNestedIe: Copy, ThisSize: Unsigned>
    MlmeIeBuilder<ThisPrevNestedIes, ThisNestedIe, ThisSize>
{
    pub fn add_short_nested_ie<NestedIe: Copy, Size: Unsigned>(
        self,
        _short_nested_ie: PhantomData<ShortNestedIe<NestedIe, Size>>,
    ) -> MlmeIeBuilder<
        MlmeIe<ThisPrevNestedIes, ThisNestedIe, Size>,
        ShortNestedIe<NestedIe, Size>,
        Sum<ThisSize, Size>,
    >
    where
        ThisSize: Add<Size>,
        Sum<ThisSize, Size>: Unsigned,
    {
        MlmeIeBuilder {
            mlme_ie: PhantomData,
        }
    }

    pub fn add_long_nested_ie<NestedIe: Copy, Size: Unsigned>(
        self,
        _long_nested_ie: PhantomData<LongNestedIe<NestedIe, Size>>,
    ) -> MlmeIeBuilder<
        MlmeIe<ThisPrevNestedIes, ThisNestedIe, Size>,
        LongNestedIe<NestedIe, Size>,
        Sum<ThisSize, Size>,
    >
    where
        ThisSize: Add<Size>,
        Sum<ThisSize, Size>: Unsigned,
    {
        MlmeIeBuilder {
            mlme_ie: PhantomData,
        }
    }

    pub fn finalize(
        self,
    ) -> PhantomData<
        PayloadIe<
            MlmeIe<ThisPrevNestedIes, ThisNestedIe, ThisSize>,
            Sum<PayloadIeHdrSize, ThisSize>,
        >,
    >
    where
        PayloadIeHdrSize: Add<ThisSize>,
        Sum<PayloadIeHdrSize, ThisSize>: Unsigned,
    {
        PhantomData
    }
}

#[bitfield(u16)]
pub struct ShortNestedIeHdr {
    #[bits(8)]
    pub length: u8,
    #[bits(7)]
    pub sub_id: NestedIeId,
    #[bits(1)]
    pub ie_type: u8, // always zero
}

#[bitfield(u16)]
pub struct LongNestedIeHdr {
    #[bits(11)]
    pub length: u16,
    #[bits(4)]
    pub sub_id: NestedIeId,
    #[bits(1)]
    pub ie_type: u8, // always one
}

pub type NestedIeHdrSize = U2;

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum NestedIeId {
    // Long Nested IEs
    ChannelHoppingIe = 0x9,

    // Short Nested IEs
    TschSynchronizationIE = 0x1a,
    TschSlotframeAndLinkIE = 0x1b,
    TschTimeslotIe = 0x1c,
}

impl NestedIeId {
    const fn into_bits(self) -> u8 {
        self as _
    }
    const fn from_bits(value: u8) -> Self {
        match value {
            0x9 => Self::ChannelHoppingIe,
            0x1a => Self::TschSynchronizationIE,
            0x1b => Self::TschSlotframeAndLinkIE,
            0x1c => Self::TschTimeslotIe,
            _ => unreachable!(),
        }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ShortNestedIe<Content, Size> {
    pub hdr: ShortNestedIeHdr,
    pub content: Content,
    size: PhantomData<Size>,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct LongNestedIe<Content: Copy, Size: Unsigned> {
    pub hdr: LongNestedIeHdr,
    pub content: Content,
    size: PhantomData<Size>,
}

#[repr(C, packed)]
#[derive(Clone, Copy, AnyBitPattern)]
pub struct ReducedChannelHoppingIeContent {
    pub hopping_seq_id: u8,
}

impl StaticallySized for ReducedChannelHoppingIeContent {
    type Size = U1;
}

#[allow(dead_code)]
pub type ReducedChannelHoppingIeSize = Sum<NestedIeHdrSize, SizeOf<ReducedChannelHoppingIeContent>>;

#[allow(dead_code)]
pub fn reduced_channel_hopping_ie(
) -> PhantomData<LongNestedIe<ReducedChannelHoppingIeContent, ReducedChannelHoppingIeSize>> {
    PhantomData
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct FullChannelHoppingIeContent<
    HoppingSeqLen: ArrayLength<ArrayType<u8>: Copy>,
    ExtBmLen: ArrayLength<ArrayType<u8>: Copy>,
> {
    pub hopping_seq_id: u8,
    pub channel_page: u8,
    pub num_channels: u16,
    pub phy_config: u32,
    pub ext_bitmap: GenericArray<u8, ExtBmLen>,
    pub hopping_seq_len: u16,
    pub hopping_seq: GenericArray<u8, HoppingSeqLen>,
    pub current_hop: u16,
}

type FullChannelHoppingIeContentFixedSize = U12;

pub struct FullChannelHoppingIeBuilder<
    HoppingSeqLen: ArrayLength<ArrayType<u8>: Copy>,
    ExtBmLen: ArrayLength<ArrayType<u8>: Copy>,
> {
    full_channel_hopping_ie: PhantomData<FullChannelHoppingIeContent<HoppingSeqLen, ExtBmLen>>,
}

impl FullChannelHoppingIeBuilder<U0, U0> {
    fn new() -> Self {
        Self {
            full_channel_hopping_ie: PhantomData,
        }
    }
}

#[allow(dead_code)]
pub fn full_channel_hopping_ie() -> FullChannelHoppingIeBuilder<U0, U0> {
    FullChannelHoppingIeBuilder::new()
}

#[allow(dead_code)]
impl<ExtBmLen: ArrayLength<ArrayType<u8>: Copy>> FullChannelHoppingIeBuilder<U0, ExtBmLen> {
    pub fn with_hopping_seq_len<const HOP_SEQ_LEN: usize>(
        self,
    ) -> FullChannelHoppingIeBuilder<U<HOP_SEQ_LEN>, ExtBmLen>
    where
        Const<HOP_SEQ_LEN>: ToUInt,
        <Const<HOP_SEQ_LEN> as ToUInt>::Output: Unsigned,
        U<HOP_SEQ_LEN>: ArrayLength<ArrayType<u8>: Copy>,
    {
        FullChannelHoppingIeBuilder {
            full_channel_hopping_ie: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<HoppingSeqLen: ArrayLength<ArrayType<u8>: Copy>>
    FullChannelHoppingIeBuilder<HoppingSeqLen, U0>
{
    pub fn with_ext_bm_len<const EXT_BM_LEN: usize>(
        self,
    ) -> FullChannelHoppingIeBuilder<HoppingSeqLen, U<EXT_BM_LEN>>
    where
        Const<EXT_BM_LEN>: ToUInt,
        <Const<EXT_BM_LEN> as ToUInt>::Output: Unsigned,
        U<EXT_BM_LEN>: ArrayLength<ArrayType<u8>: Copy>,
    {
        FullChannelHoppingIeBuilder {
            full_channel_hopping_ie: PhantomData,
        }
    }
}

#[allow(dead_code)]
impl<
        HoppingSeqLen: ArrayLength<ArrayType<u8>: Copy>,
        ExtBmLen: ArrayLength<ArrayType<u8>: Copy>,
    > FullChannelHoppingIeBuilder<HoppingSeqLen, ExtBmLen>
{
    pub fn finalize(
        self,
    ) -> PhantomData<
        LongNestedIe<
            FullChannelHoppingIeContent<HoppingSeqLen, ExtBmLen>,
            Sum<
                NestedIeHdrSize,
                Sum<Sum<FullChannelHoppingIeContentFixedSize, HoppingSeqLen>, ExtBmLen>,
            >,
        >,
    >
    where
        NestedIeHdrSize:
            Add<Sum<Sum<FullChannelHoppingIeContentFixedSize, HoppingSeqLen>, ExtBmLen>>,
        FullChannelHoppingIeContentFixedSize: Add<HoppingSeqLen>,
        Sum<FullChannelHoppingIeContentFixedSize, HoppingSeqLen>: Add<ExtBmLen>,
        Sum<
            NestedIeHdrSize,
            Sum<Sum<FullChannelHoppingIeContentFixedSize, HoppingSeqLen>, ExtBmLen>,
        >: Unsigned,
    {
        PhantomData
    }
}

#[derive(Clone, Copy)]
pub struct ParsedFullChannelHoppingIeContent<'frame> {
    content: &'frame [u8],
}

#[repr(u8)]
pub enum TestVariant {
    Opt1,
    Opt2,
    Opt3,
    Opt4,
    Opt5,
}

impl<'frame> ParsedFullChannelHoppingIeContent<'frame> {
    const HOPPING_SEQ_ID_OFFSET: usize = 0;
    const HOPPING_SEQ_ID_LEN: usize = 1;
    const CHANNEL_PAGE_OFFSET: usize = Self::HOPPING_SEQ_ID_OFFSET + Self::HOPPING_SEQ_ID_LEN;
    const CHANNEL_PAGE_LEN: usize = 1;
    const NUM_CHANNELS_OFFSET: usize = Self::CHANNEL_PAGE_OFFSET + Self::CHANNEL_PAGE_LEN;
    const NUM_CHANNELS_LEN: usize = 2;
    const PHY_CONFIG_OFFSET: usize = Self::NUM_CHANNELS_OFFSET + Self::NUM_CHANNELS_LEN;
    const PHY_CONFIG_LEN: usize = 4;
    const EXTENDED_BITMAP_OFFSET: usize = Self::PHY_CONFIG_OFFSET + Self::PHY_CONFIG_LEN;
    const HOPPING_SEQ_LEN_LEN: usize = 2;
    const CURRENT_HOP_LEN: usize = 2;

    pub fn hopping_seq_id(&self) -> u8 {
        self.content[Self::HOPPING_SEQ_ID_OFFSET]
    }

    pub fn channel_page(&self) -> u8 {
        self.content[Self::CHANNEL_PAGE_OFFSET]
    }

    pub fn num_channels(&self) -> u16 {
        LE::read_u16(
            &self.content
                [Self::NUM_CHANNELS_OFFSET..(Self::NUM_CHANNELS_OFFSET + Self::NUM_CHANNELS_LEN)],
        )
    }

    pub fn phy_config(&self) -> u32 {
        LE::read_u32(
            &self.content
                [Self::PHY_CONFIG_OFFSET..(Self::PHY_CONFIG_OFFSET + Self::PHY_CONFIG_LEN)],
        )
    }

    pub fn ext_bitmap<ExtBmLen: ArrayLength>(&self) -> &'frame [u8] {
        &self.content[Self::EXTENDED_BITMAP_OFFSET
            ..(Self::EXTENDED_BITMAP_OFFSET + self.ext_bitmap_len::<ExtBmLen>())]
    }

    pub fn hopping_seq<ExtBmLen: ArrayLength>(&self) -> &'frame [u8] {
        let hopping_seq_offset = self.hopping_seq_offset::<ExtBmLen>();
        &self.content[hopping_seq_offset..(hopping_seq_offset + self.hopping_seq_len::<ExtBmLen>())]
    }

    pub fn current_hop<ExtBmLen: ArrayLength>(&self) -> u16 {
        let current_hop_offset = self.current_hop_offset::<ExtBmLen>();
        LE::read_u16(
            &self.content[current_hop_offset..(current_hop_offset + Self::CURRENT_HOP_LEN)],
        )
    }

    fn ext_bitmap_len<ExtBmLen: ArrayLength>(&self) -> usize {
        <ExtBmLen as Unsigned>::to_usize()
    }

    fn hopping_seq_len_offset<ExtBmLen: ArrayLength>(&self) -> usize {
        Self::EXTENDED_BITMAP_OFFSET + self.ext_bitmap_len::<ExtBmLen>()
    }

    fn hopping_seq_len<ExtBmLen: ArrayLength>(&self) -> usize {
        let hopping_seq_len_offset = self.hopping_seq_len_offset::<ExtBmLen>();
        LE::read_u16(
            &self.content
                [hopping_seq_len_offset..(hopping_seq_len_offset + Self::HOPPING_SEQ_LEN_LEN)],
        ) as usize
    }

    fn hopping_seq_offset<ExtBmLen: ArrayLength>(&self) -> usize {
        self.hopping_seq_len_offset::<ExtBmLen>() + Self::HOPPING_SEQ_LEN_LEN
    }

    fn current_hop_offset<ExtBmLen: ArrayLength>(&self) -> usize {
        self.hopping_seq_offset::<ExtBmLen>() + self.hopping_seq_len::<ExtBmLen>()
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy, AnyBitPattern)]
pub struct TschSynchronizationIeContent {
    pub asn: [u8; 5],
    pub join_metric: u8,
}

impl StaticallySized for TschSynchronizationIeContent {
    type Size = U6;
}

#[allow(dead_code)]
pub type TschSynchronizationNestedIe = ShortNestedIe<
    TschSynchronizationIeContent,
    Sum<NestedIeHdrSize, SizeOf<TschSynchronizationIeContent>>,
>;

#[bitfield(u8)]
#[derive(AnyBitPattern)]
pub struct TschLinkOptions {
    #[bits(1)]
    pub tx_link: bool,
    #[bits(1)]
    pub rx_link: bool,
    #[bits(1)]
    pub shared_link: bool,
    #[bits(1)]
    pub timekeeping: bool,
    #[bits(1)]
    pub priority: bool,
    #[bits(3)]
    pub reserved: u8,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct TschLinkInformation {
    pub timeslot: u16,
    pub channel_offset: u16,
    pub link_options: TschLinkOptions,
}

#[repr(C, packed)]
#[derive(Clone, Copy, AnyBitPattern)]
pub struct ParsedTschLinkInformation {
    pub timeslot: u16,
    pub channel_offset: u16,
    pub link_options: TschLinkOptions,
}

//noinspection RsAssertEqual
const _: () = assert!(size_of::<ParsedTschLinkInformation>() == 5);

#[repr(C, packed)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct TschSlotframeDescriptor<NumLinks: ArrayLength<ArrayType<TschLinkInformation>: Copy>> {
    pub slotframes_handle: u8,
    pub slotframe_size: u16,
    pub num_links: u8,
    pub link_information: GenericArray<TschLinkInformation, NumLinks>,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct TschSlotframeDescriptors<PrevSlotframeDescriptors: Copy, SlotframeDescriptor: Copy> {
    pub prev: PrevSlotframeDescriptors,
    pub slotframe_descriptor: SlotframeDescriptor,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct ParsedTschSlotframeDescriptor<'frame> {
    content: &'frame [u8],
}

#[allow(dead_code)]
impl<'frame> ParsedTschSlotframeDescriptor<'frame> {
    const SLOTFRAMES_HANDLE_OFFSET: usize = 0;
    const SLOTFRAMES_HANDLE_LEN: usize = 1;
    const SLOTFRAME_SIZE_OFFSET: usize =
        Self::SLOTFRAMES_HANDLE_OFFSET + Self::SLOTFRAMES_HANDLE_LEN;
    const SLOTFRAME_SIZE_LEN: usize = 2;
    const NUM_LINKS_OFFSET: usize = Self::SLOTFRAME_SIZE_OFFSET + Self::SLOTFRAME_SIZE_LEN;
    const NUM_LINKS_LEN: usize = 1;
    const LINK_INFO_OFFSET: usize = Self::NUM_LINKS_OFFSET + Self::NUM_LINKS_LEN;

    // Returns the size of the next slotframe descriptor given a buffer that
    // contains a slotframe descriptor list.
    pub fn size(buffer: &[u8]) -> usize {
        Self::SLOTFRAMES_HANDLE_LEN
            + Self::SLOTFRAME_SIZE_LEN
            + Self::NUM_LINKS_LEN
            + (buffer[Self::NUM_LINKS_OFFSET] as usize) * size_of::<ParsedTschLinkInformation>()
    }

    pub fn slotframes_handle(&self) -> u8 {
        self.content[Self::SLOTFRAMES_HANDLE_OFFSET]
    }

    pub fn slotframe_size(&self) -> u16 {
        LE::read_u16(
            &self.content[Self::SLOTFRAME_SIZE_OFFSET
                ..(Self::SLOTFRAMES_HANDLE_OFFSET + Self::SLOTFRAME_SIZE_LEN)],
        )
    }

    pub fn num_links(&self) -> usize {
        self.content[Self::NUM_LINKS_OFFSET] as usize
    }

    pub fn link_information(&self) -> &'frame [ParsedTschLinkInformation] {
        // NOTE: Parsed link information is 1-packed, i.e. 1-aligned and
        //       therefore the following call will always succeed if the content
        //       length is valid.
        bytemuck::cast_slice::<u8, ParsedTschLinkInformation>(
            &self.content[Self::LINK_INFO_OFFSET
                ..(Self::LINK_INFO_OFFSET
                    + self.num_links() * size_of::<ParsedTschLinkInformation>())],
        )
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct TschSlotframeAndLinkIe<SlotframeDescriptors: Copy> {
    pub num_slotframes: u8,
    pub slotframe_descriptors: SlotframeDescriptors,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct TschSlotframeAndLinkIeBuilder<SlotframeDescriptors: Copy, Size: Unsigned> {
    slotframe_descriptors: PhantomData<SlotframeDescriptors>,
    size: PhantomData<Size>,
}

#[allow(dead_code)]
impl TschSlotframeAndLinkIeBuilder<(), U0> {
    fn new() -> Self {
        Self {
            slotframe_descriptors: PhantomData,
            size: PhantomData,
        }
    }

    pub fn add_slotframe_descriptor(
        self,
    ) -> TschSlotframeAndLinkIeBuilder<TschSlotframeDescriptors<(), TschSlotframeDescriptor<U0>>, U4>
    {
        TschSlotframeAndLinkIeBuilder {
            slotframe_descriptors: PhantomData,
            size: PhantomData,
        }
    }
}

#[allow(dead_code)]
pub fn tsch_slotframe_and_link_ie() -> TschSlotframeAndLinkIeBuilder<(), U0> {
    TschSlotframeAndLinkIeBuilder::new()
}

#[allow(dead_code)]
impl<ThisPrevSlotframeDescriptors: Copy, ThisSlotframeDescriptor: Copy, Size: Unsigned>
    TschSlotframeAndLinkIeBuilder<
        TschSlotframeDescriptors<ThisPrevSlotframeDescriptors, ThisSlotframeDescriptor>,
        Size,
    >
{
    pub fn add_slotframe_descriptor(
        self,
    ) -> TschSlotframeAndLinkIeBuilder<
        TschSlotframeDescriptors<
            TschSlotframeDescriptors<ThisPrevSlotframeDescriptors, ThisSlotframeDescriptor>,
            TschSlotframeDescriptor<U0>,
        >,
        Sum<Size, U4>,
    >
    where
        Size: Add<U4>,
        Sum<Size, U4>: Unsigned,
    {
        TschSlotframeAndLinkIeBuilder {
            slotframe_descriptors: PhantomData,
            size: PhantomData,
        }
    }

    pub fn finalize(
        self,
    ) -> PhantomData<
        ShortNestedIe<
            TschSlotframeAndLinkIe<
                TschSlotframeDescriptors<ThisPrevSlotframeDescriptors, ThisSlotframeDescriptor>,
            >,
            Sum<Sum<Size, U1>, NestedIeHdrSize>,
        >,
    >
    where
        Size: Add<U1>,
        Sum<Size, U1>: Add<NestedIeHdrSize>,
        Sum<Sum<Size, U1>, NestedIeHdrSize>: Unsigned,
    {
        PhantomData
    }
}

#[allow(dead_code)]
impl<PrevSlotframeDescriptors: Copy, Size: Unsigned>
    TschSlotframeAndLinkIeBuilder<
        TschSlotframeDescriptors<PrevSlotframeDescriptors, TschSlotframeDescriptor<U0>>,
        Size,
    >
{
    pub fn with_num_links<const NUM_LINKS: usize>(
        self,
    ) -> TschSlotframeAndLinkIeBuilder<
        TschSlotframeDescriptors<PrevSlotframeDescriptors, TschSlotframeDescriptor<U<NUM_LINKS>>>,
        Sum<Size, Prod<U5, U<NUM_LINKS>>>,
    >
    where
        Const<NUM_LINKS>: ToUInt,
        <Const<NUM_LINKS> as ToUInt>::Output: ArrayLength<ArrayType<TschLinkInformation>: Copy>,
        U5: Mul<U<NUM_LINKS>>,
        Size: Add<Prod<U5, U<NUM_LINKS>>>,
        Sum<Size, Prod<U5, U<NUM_LINKS>>>: Unsigned,
    {
        TschSlotframeAndLinkIeBuilder {
            slotframe_descriptors: PhantomData,
            size: PhantomData,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct ParsedTschSlotframeAndLinkIeContent<'frame> {
    content: &'frame [u8],
}

#[allow(dead_code)]
impl ParsedTschSlotframeDescriptor<'_> {
    const NUM_SLOTFRAMES_OFFSET: usize = 0;
    const NUM_SLOTFRAMES_LEN: usize = 1;

    pub fn slotframe_descriptors(&self) -> ParsedSlotframeDescriptorIterator {
        ParsedSlotframeDescriptorIterator {
            content: &self.content[Self::NUM_SLOTFRAMES_LEN..],
        }
    }

    pub fn number_of_slotframes(&self) -> u8 {
        self.content[Self::NUM_SLOTFRAMES_OFFSET]
    }
}

pub struct ParsedSlotframeDescriptorIterator<'f> {
    content: &'f [u8],
}

impl<'frame> ParsedSlotframeDescriptorIterator<'frame> {
    fn terminated(&self) -> bool {
        self.content.len() == 0
    }
}

impl<'frame> Iterator for ParsedSlotframeDescriptorIterator<'frame> {
    type Item = ParsedTschSlotframeDescriptor<'frame>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.terminated() {
            return None;
        }

        let next_slotframe_size = ParsedTschSlotframeDescriptor::size(self.content);

        let (remaining_content, next_slotframe) = self.content.split_at(next_slotframe_size);
        self.content = remaining_content;

        let descriptor = ParsedTschSlotframeDescriptor {
            content: next_slotframe,
        };

        Some(descriptor)
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy, AnyBitPattern)]
pub struct ReducedTschTimeslotIeContent {
    pub timeslot_id: u8,
}

#[allow(dead_code)]
pub fn reduced_tsch_timeslot_ie(
) -> PhantomData<ShortNestedIe<ReducedTschTimeslotIeContent, Sum<NestedIeHdrSize, U1>>> {
    PhantomData
}

#[repr(C, packed)]
#[derive(Clone, Copy, AnyBitPattern)]
pub struct FullTschTimeslotIeContent {
    pub timeslot_id: u8,
    pub cca_offset: u16,
    pub cca: u16,
    pub tx_offset: u16,
    pub rx_offset: u16,
    pub rx_ack_delay: u16,
    pub tx_ack_delay: u16,
    pub rx_wait: u16,
    pub ack_wait: u16,
    pub max_ack: u16,
    pub max_tx_high: u8, // TODO: The spec has a variant with 2 bytes for max_tx/timeslot_length.
    pub max_tx_low: u16,
    pub timeslot_length_high: u8,
    pub timeslot_length_low: u16,
}

#[allow(dead_code)]
pub type FullTschTimeslotNestedIe = ShortNestedIe<FullTschTimeslotIeContent, U27>;

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct PayloadTerminationIe;

#[allow(dead_code)]
pub fn payload_termination_ie() -> PhantomData<PayloadIe<(), PayloadIeHdrSize>> {
    PhantomData
}

pub enum ParsedPayloadIe<'frame> {
    MlmeIe(&'frame [u8]),
    PayloadTerminationIe,
}

pub fn parse_payload_ie(buffer: &[u8]) -> Result<(&[u8], ParsedPayloadIe), ()> {
    let ie_hdr = PayloadIeHdr::from_bits(LE::read_u16(buffer));
    let buffer = &buffer[2..];

    if ie_hdr.ie_type() != 1 {
        return Err(());
    }

    let (buffer, parsed_ie) = match ie_hdr.group_id() {
        PayloadIeGroupId::MlmeIe => (
            &buffer[(ie_hdr.length() as usize)..],
            ParsedPayloadIe::MlmeIe(buffer),
        ),
        PayloadIeGroupId::PayloadTerminationIe => (buffer, ParsedPayloadIe::PayloadTerminationIe),
    };

    Ok((buffer, parsed_ie))
}

pub fn parse_nested_ie(buffer: &[u8]) -> Result<(&[u8], ParsedIe), ()> {
    let is_long_nested_ie = buffer[0] & 0b1000_0000 > 0;

    let (length, element_id) = if is_long_nested_ie {
        let ie_hdr = LongNestedIeHdr::from_bits(LE::read_u16(buffer));
        (ie_hdr.length(), ie_hdr.sub_id())
    } else {
        let ie_hdr = ShortNestedIeHdr::from_bits(LE::read_u16(buffer));
        (ie_hdr.length() as u16, ie_hdr.sub_id())
    };

    let (buffer, ie_content) =
        buffer[<NestedIeHdrSize as Unsigned>::to_usize()..].split_at(length as usize);

    let parsed_ie = match element_id {
        NestedIeId::ChannelHoppingIe => parse_channel_hopping_ie(ie_content),
        NestedIeId::TschSynchronizationIE => parse_tsch_synchronization_ie(ie_content),
        NestedIeId::TschSlotframeAndLinkIE => parse_tsch_slotframe_and_link_ie(ie_content),
        NestedIeId::TschTimeslotIe => parse_tsch_timeslot_ie(ie_content),
    };

    Ok((buffer, parsed_ie))
}

fn parse_channel_hopping_ie(content: &[u8]) -> ParsedIe<'_> {
    if content.len() == 1 {
        return ParsedIe::ReducedChannelHoppingIe(bytemuck::from_bytes::<
            ReducedChannelHoppingIeContent,
        >(content));
    }

    ParsedIe::FullChannelHoppingIe(ParsedFullChannelHoppingIeContent { content })
}

fn parse_tsch_synchronization_ie(content: &[u8]) -> ParsedIe<'_> {
    ParsedIe::TschSynchronizationIe(bytemuck::from_bytes(content))
}

fn parse_tsch_slotframe_and_link_ie(content: &[u8]) -> ParsedIe<'_> {
    ParsedIe::TschSlotframeAndLinkIe(ParsedTschSlotframeAndLinkIeContent { content })
}

fn parse_tsch_timeslot_ie(content: &[u8]) -> ParsedIe<'_> {
    if content.len() == 1 {
        return ParsedIe::ReducedTschTimeslotIe(bytemuck::from_bytes(content));
    }

    ParsedIe::FullTschTimeslotIe(bytemuck::from_bytes(content))
}
