use core::marker::PhantomData;
use core::ops::{Add, Sub};

use bytemuck::{AnyBitPattern, NoUninit, Zeroable};
use crc::{CRC_16_KERMIT, CRC_32_ISO_HDLC};
use generic_array::ArrayLength;
use typenum::{Const, Diff, Sum, ToUInt, Unsigned, U, U0, U1, U3};

use crate::addr::{Address, NoAddressing, ParsedAddressing};
use crate::driver::{self, Fcs2Byte, Fcs4Byte, NoFcs, RadioDriverConfig};
use crate::fc::{fc_ieee802154, fc_ieee802154_2006, FrameType};
use crate::header_ie::{NoHeaderIes, TimeCorrectionIeContent};
use crate::payload_ie::{
    FullTschTimeslotIeContent, NoPayloadIes, ParsedFullChannelHoppingIeContent,
    ParsedTschSlotframeAndLinkIeContent, ReducedChannelHoppingIeContent,
    ReducedTschTimeslotIeContent, TschSynchronizationIeContent,
};
use crate::pib::MacPib;
use crate::sec::{
    aux_sec_hdr, lookup_key_descriptor, NoAuxSecHeader, ParsedAuxSecHeader, SecurityContext,
};
use crate::{fc::FrameControl, StaticallySized};
use crate::{GenericByteBuffer, SizeOf};

type DriverHeadroom<Spec> = driver::Headroom<<Spec as FrameSpec>::Driver>;
type DriverTailroom<Spec> = driver::Tailroom<<Spec as FrameSpec>::Driver>;
type FcsType<Spec> = driver::FcsType<<Spec as FrameSpec>::Driver>;

pub trait FrameSpec {
    type Driver: RadioDriverConfig + Copy;
    type SeqNum: StaticallySized + Copy;
    type Addressing: StaticallySized + Copy;
    type AuxSecHeader: StaticallySized + Copy;
    type HeaderIes: StaticallySized + Copy;
    type PayloadIes: StaticallySized + Copy;
    type FramePayloadSize: ArrayLength<ArrayType<u8>: Copy>;
    type MpduSize: Unsigned;
}

// types allowed for FrameSpec::SeqNum
pub type WithSeqNum = u8;
pub type WithoutSeqNum = ();

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MacHeader<Spec: FrameSpec> {
    pub fc: FrameControl,
    pub seq_num: <Spec as FrameSpec>::SeqNum,
    pub addressing: <Spec as FrameSpec>::Addressing,
    pub aux_sec_hdr: <Spec as FrameSpec>::AuxSecHeader,
    pub header_ies: <Spec as FrameSpec>::HeaderIes,
}

// NOTE: We cannot use Sum<SizeOf<FrameControlSize>, SizeOf<SeqNumSize>> here as it will be
//       evaluated in `where` context but not so in generic type context.
pub type MinMacHeaderSize = U3;

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MacPayload<Spec: FrameSpec> {
    pub payload_ies: <Spec as FrameSpec>::PayloadIes,
    pub frame_payload: GenericByteBuffer<<Spec as FrameSpec>::FramePayloadSize>,
}

#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct MacFooter<Fcs> {
    pub fcs: Fcs,
}

pub trait WithCrc {
    fn set_from_bytes(&mut self, bytes: &[u8]);
}

impl WithCrc for MacFooter<NoFcs> {
    fn set_from_bytes(&mut self, _bytes: &[u8]) {
        // no-op
    }
}

impl MacFooter<Fcs2Byte> {
    pub(super) const fn get_crc_impl(self) -> crc::Crc<Fcs2Byte> {
        // The FCS field contains a 16-bit ITU-T CRC (aka CRC-16/KERMIT, see
        // https://reveng.sourceforge.io/crc-catalogue/16.htm).
        crc::Crc::<Fcs2Byte>::new(&CRC_16_KERMIT)
    }
}

impl WithCrc for MacFooter<Fcs2Byte> {
    fn set_from_bytes(&mut self, bytes: &[u8]) {
        self.fcs = self.get_crc_impl().checksum(bytes);
    }
}

impl MacFooter<Fcs4Byte> {
    pub(super) const fn get_crc_impl(self) -> crc::Crc<Fcs4Byte> {
        // The FCS field contains a 32-bit ANSI X3.66-1979 CRC (aka CRC-32/ISO-HDLC,
        // see https://reveng.sourceforge.io/crc-catalogue/17plus.htm).
        crc::Crc::<Fcs4Byte>::new(&CRC_32_ISO_HDLC)
    }
}

impl WithCrc for MacFooter<Fcs4Byte> {
    fn set_from_bytes(&mut self, bytes: &[u8]) {
        if bytes.len() < 4 {
            let mut padded_bytes = [0; 4];
            padded_bytes[..bytes.len()].copy_from_slice(bytes);
            self.fcs = self.get_crc_impl().checksum(&padded_bytes);
        } else {
            self.fcs = self.get_crc_impl().checksum(bytes);
        }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Mpdu<Spec: FrameSpec> {
    pub mhr: MacHeader<Spec>,
    pub mac_payload: MacPayload<Spec>,
    pub mfr: MacFooter<FcsType<Spec>>,
}

// SAFETY
// The type does not contain any padding and will be valid with all zeroes.
// See Ppdu::new().
unsafe impl<Spec: FrameSpec> Zeroable for Mpdu<Spec> {}

// SAFETY
// See safety conditions of AnyBitPattern - the relevant ones being repeated here:
// * The type is inhabited.
// * `repr(C, packed)` ensures that the type does not contain padding.
// * The struct does not contain pointer types, atomics, or any other form of
//   interior mutability.
unsafe impl<Spec: FrameSpec + Copy + 'static> AnyBitPattern for Mpdu<Spec> {}

// SAFETY
//
// See safety conditions of NoUninit - the relevant ones being repeated here:
// * The type is inhabited.
// * `repr(C, packed)` ensures that the type does not contain padding.
// * The struct does not contain pointer types, atomics, or any other form of
//   interior mutability.
unsafe impl<Spec: FrameSpec + Copy + 'static> NoUninit for Mpdu<Spec> {}

#[allow(dead_code)]
impl<Spec: FrameSpec + Copy + 'static> Mpdu<Spec> {
    pub(super) fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Ppdu<Spec: FrameSpec> {
    // hardware specific headroom, may include (parts of) the PHY header.
    pub headroom: GenericByteBuffer<DriverHeadroom<Spec>>,
    pub mpdu: Mpdu<Spec>,
    // : hardware specific tailroom
    pub tailroom: GenericByteBuffer<DriverTailroom<Spec>>,
}

// SAFETY
// The type does not contain any padding and will be valid with all zeroes.
// See Ppdu::new().
unsafe impl<Spec: FrameSpec> Zeroable for Ppdu<Spec> {}

// SAFETY
// See safety conditions of AnyBitPattern - the relevant ones being repeated here:
// * The type is inhabited.
// * IEEE 802.15.4 frames (and therefore all sub-fields of an MPDU are basically
//   valid for any bit pattern. Any invalid patterns SHALL be checked at runtime
//   by appropriate setters/getters.
// * Headroom and Tailroom is basically a u8 array which is AnyBitPattern, too.
//   Any deviating conditions SHALL be checked by drivers at runtime.
// * `repr(C, packed)` ensures that the type does not contain padding.
// * The struct does not contain pointer types, atomics, or any other form of
//   interior mutability.
unsafe impl<Spec: FrameSpec + Copy + 'static> AnyBitPattern for Ppdu<Spec> {}

// SAFETY: See NoUninit
// * The type is inhabited.
// * `repr(C, packed)` ensures that the type does not contain padding.
// * The struct does not contain pointer types, atomics, or any other form of
//   interior mutability.
unsafe impl<Spec: FrameSpec + Copy + 'static> NoUninit for Ppdu<Spec> {}

#[allow(dead_code)]
impl<Spec: FrameSpec + Copy + 'static> Ppdu<Spec> {
    pub fn new() -> Self {
        Self::zeroed()
    }

    pub fn from_buffer(buffer: &[u8]) -> &Ppdu<Spec> {
        bytemuck::from_bytes(buffer)
    }

    /// Calculate the Frame Check Sequence (FCS) of the frame.
    ///
    /// The CRC is calculated over the entire MPDU, excluding the FCS field
    /// itself.
    ///
    /// NOTE: This is a no-op if the driver offloads CRC calculation.
    pub fn set_fcs(&mut self)
    where
        MacFooter<FcsType<Spec>>: WithCrc,
    {
        let mpdu = self.mpdu;
        let mpdu_as_bytes = mpdu.as_bytes();
        let mpdu_without_fcs = &mpdu_as_bytes[..(mpdu_as_bytes.len() - size_of::<FcsType<Spec>>())];
        let mut mfr = self.mpdu.mfr;
        mfr.set_from_bytes(mpdu_without_fcs);
        self.mpdu.mfr = mfr;
    }
}

#[allow(dead_code)]
impl<Spec: FrameSpec + Copy + 'static> Ppdu<Spec> {
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

#[derive(Clone, Copy)]
pub struct FrameSpecImpl<
    Driver: RadioDriverConfig + Copy,
    SeqNum: StaticallySized + Copy = WithSeqNum,
    Addressing: StaticallySized + Copy = NoAddressing,
    AuxSecHeader: StaticallySized + Copy = NoAuxSecHeader,
    HeaderIes: StaticallySized + Copy = NoHeaderIes,
    PayloadIes: StaticallySized + Copy = NoPayloadIes,
    FramePayloadSize: ArrayLength<ArrayType<u8>: Copy> = U0,
    MpduSize: Unsigned = Sum<MinMacHeaderSize, driver::FcsSize<Driver>>,
> {
    frame: PhantomData<Ppdu<Self>>,
}

impl<
        Driver: RadioDriverConfig + Copy,
        SeqNum: StaticallySized + Copy,
        Addressing: StaticallySized + Copy,
        AuxSecHeader: StaticallySized + Copy,
        HeaderIes: StaticallySized + Copy,
        PayloadIes: StaticallySized + Copy,
        FramePayloadSize: ArrayLength<ArrayType<u8>: Copy>,
        MpduSize: Unsigned,
    > FrameSpec
    for FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        MpduSize,
    >
{
    type Driver = Driver;
    type SeqNum = SeqNum;
    type Addressing = Addressing;
    type AuxSecHeader = AuxSecHeader;
    type HeaderIes = HeaderIes;
    type PayloadIes = PayloadIes;
    type FramePayloadSize = FramePayloadSize;
    type MpduSize = MpduSize;
}

#[allow(dead_code)]
impl<Driver: RadioDriverConfig + Copy + 'static> FrameSpecImpl<Driver>
where
    MinMacHeaderSize: Add<driver::FcsSize<Driver>>,
    Sum<MinMacHeaderSize, driver::FcsSize<Driver>>: Unsigned,
{
    fn new(_driver_config: Driver) -> Self {
        Self { frame: PhantomData }
    }

    pub fn imm_ack(self, seq_num: u8) -> Ppdu<Self> {
        let mut ack = self.finalize();

        ack.mpdu.mhr.fc = fc_ieee802154_2006(FrameType::Acknowledgment);
        ack.mpdu.mhr.seq_num = seq_num;

        ack
    }

    pub fn enh_ack(
        self,
        _src_addr: Address,
        dst_addr: Address,
        pan_id: Option<u16>,
        seq_num: u8,
        frame_pending: bool,
        security_context: SecurityContext,
        mac_pib: &MacPib,
    ) -> Result<Ppdu<FrameSpecImpl<Driver>>, FrameError>
    where
        MacFooter<FcsType<Self>>: WithCrc,
    {
        let mut fc = fc_ieee802154(FrameType::Acknowledgment);

        // See IEEE 802.15.4-2020, section 9.2.2
        match security_context.security_level {
            // a) Is security needed?
            crate::sec::SecurityLevel::None => {}
            _security_level => {
                // b) Is security enabled?
                if !mac_pib.mac_security_enabled {
                    return Err(FrameError::UnsupportedSecurity);
                }

                fc.set_security_enabled(true);

                // c) Obtain KeyDescriptor.
                let key_descriptor =
                    match lookup_key_descriptor(security_context.key_id, pan_id, dst_addr) {
                        Some(key_descriptor) => key_descriptor,
                        None => return Err(FrameError::UnavailableKey),
                    };

                // d) Check frame counter value.
                if !mac_pib.mac_tsch_enabled {
                    if key_descriptor.sec_frame_counter_per_key {
                        if key_descriptor.sec_key_frame_counter == 0xffffffff {
                            return Err(FrameError::CounterError);
                        }
                    } else {
                        if mac_pib.sec_frame_counter == 0xffffffff {
                            return Err(FrameError::CounterError);
                        }
                    }
                }

                // e) Insert Auxiliary Security Header field.
                let aux_sec_header = aux_sec_hdr();

                if !mac_pib.mac_tsch_enabled {
                    let _aux_sec_header = aux_sec_header.with_frame_counter();
                }

                /* if let KeyIdVariant::Implicit = security_context.key_id {
                } else {
                    let aux_sec_header = aux_sec_header.with_key_id();
                }

                self = self.with_aux_sec_header(aux_sec_header); */
            }
        }

        if frame_pending {
            if mac_pib.mac_le_hs_enabled {
                fc.set_frame_pending(true);
            } else {
                panic!("macLeHsEnabled==false")
            }
        }

        let mut ack = self.finalize();

        ack.mpdu.mhr.fc = fc;
        ack.mpdu.mhr.seq_num = seq_num;

        ack.set_fcs();
        Ok(ack)
    }
}

pub enum FrameError {
    UnsupportedSecurity,
    UnavailableKey,
    CounterError,
}

#[allow(dead_code)]
pub fn ieee802154_frame<Driver: RadioDriverConfig + Copy + 'static>(
    driver_config: Driver,
) -> FrameSpecImpl<Driver>
where
    MinMacHeaderSize: Add<driver::FcsSize<Driver>>,
    Sum<MinMacHeaderSize, driver::FcsSize<Driver>>: Unsigned,
{
    FrameSpecImpl::new(driver_config)
}

#[allow(dead_code)]
impl<
        Driver: RadioDriverConfig + Copy,
        Addressing: StaticallySized + Copy,
        AuxSecHeader: StaticallySized + Copy,
        HeaderIes: StaticallySized + Copy,
        PayloadIes: StaticallySized + Copy,
        FramePayloadSize: ArrayLength<ArrayType<u8>: Copy>,
        Size: Unsigned,
    >
    FrameSpecImpl<
        Driver,
        WithSeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        Size,
    >
{
    pub fn without_seq_num(
        self,
    ) -> FrameSpecImpl<
        Driver,
        WithoutSeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        Diff<Size, U1>,
    >
    where
        Size: Sub<U1>,
        Diff<Size, U1>: Unsigned,
    {
        FrameSpecImpl { frame: PhantomData }
    }
}

#[allow(dead_code)]
impl<
        Driver: RadioDriverConfig + Copy,
        SeqNum: StaticallySized + Copy,
        AuxSecHeader: StaticallySized + Copy,
        HeaderIes: StaticallySized + Copy,
        PayloadIes: StaticallySized + Copy,
        FramePayloadSize: ArrayLength<ArrayType<u8>: Copy>,
        MpduSize: Unsigned,
    >
    FrameSpecImpl<
        Driver,
        SeqNum,
        NoAddressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        MpduSize,
    >
{
    pub fn with_addressing<Addressing: StaticallySized + Copy>(
        self,
        _addressing: PhantomData<Addressing>,
    ) -> FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        Sum<MpduSize, SizeOf<Addressing>>,
    >
    where
        MpduSize: Add<SizeOf<Addressing>>,
        Sum<MpduSize, SizeOf<Addressing>>: Unsigned,
    {
        FrameSpecImpl { frame: PhantomData }
    }
}

#[allow(dead_code)]
impl<
        Driver: RadioDriverConfig + Copy,
        SeqNum: StaticallySized + Copy,
        Addressing: StaticallySized + Copy,
        HeaderIes: StaticallySized + Copy,
        PayloadIes: StaticallySized + Copy,
        FramePayloadSize: ArrayLength<ArrayType<u8>: Copy>,
        MpduSize: Unsigned,
    >
    FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        NoAuxSecHeader,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        MpduSize,
    >
{
    pub fn with_aux_sec_header<AuxSecHeader: StaticallySized + Copy>(
        self,
        _aux_sec_header: PhantomData<AuxSecHeader>,
    ) -> FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        Sum<MpduSize, SizeOf<AuxSecHeader>>,
    >
    where
        MpduSize: Add<SizeOf<AuxSecHeader>>,
        Sum<MpduSize, SizeOf<AuxSecHeader>>: Unsigned,
    {
        FrameSpecImpl { frame: PhantomData }
    }
}

#[allow(dead_code)]
impl<
        Driver: RadioDriverConfig + Copy,
        SeqNum: StaticallySized + Copy,
        Addressing: StaticallySized + Copy,
        AuxSecHeader: StaticallySized + Copy,
        PayloadIes: StaticallySized + Copy,
        FramePayloadSize: ArrayLength<ArrayType<u8>: Copy>,
        MpduSize: Unsigned,
    >
    FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        AuxSecHeader,
        NoHeaderIes,
        PayloadIes,
        FramePayloadSize,
        MpduSize,
    >
{
    pub fn with_header_ies<HeaderIes: StaticallySized + Copy>(
        self,
        _header_ies: PhantomData<HeaderIes>,
    ) -> FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        Sum<MpduSize, SizeOf<HeaderIes>>,
    >
    where
        MpduSize: Add<SizeOf<HeaderIes>>,
        Sum<MpduSize, SizeOf<HeaderIes>>: Unsigned,
    {
        FrameSpecImpl { frame: PhantomData }
    }
}

#[allow(dead_code)]
impl<
        Driver: RadioDriverConfig + Copy,
        SeqNum: StaticallySized + Copy,
        Addressing: StaticallySized + Copy,
        AuxSecHeader: StaticallySized + Copy,
        HeaderIes: StaticallySized + Copy,
        FramePayloadSize: ArrayLength<ArrayType<u8>: Copy>,
        MpduSize: Unsigned,
    >
    FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        NoPayloadIes,
        FramePayloadSize,
        MpduSize,
    >
{
    pub fn with_payload_ies<PayloadIes: StaticallySized + Copy>(
        self,
        _payload_ies: PhantomData<PayloadIes>,
    ) -> FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        Sum<MpduSize, SizeOf<PayloadIes>>,
    >
    where
        MpduSize: Add<SizeOf<PayloadIes>>,
        Sum<MpduSize, SizeOf<PayloadIes>>: Unsigned,
    {
        FrameSpecImpl { frame: PhantomData }
    }
}

#[allow(dead_code)]
impl<
        Driver: RadioDriverConfig + Copy,
        SeqNum: StaticallySized + Copy,
        Addressing: StaticallySized + Copy,
        AuxSecHeader: StaticallySized + Copy,
        HeaderIes: StaticallySized + Copy,
        PayloadIes: StaticallySized + Copy,
        Size: Unsigned,
    > FrameSpecImpl<Driver, SeqNum, Addressing, AuxSecHeader, HeaderIes, PayloadIes, U0, Size>
{
    pub fn with_frame_payload_size<const FRAME_PAYLOAD_SIZE: usize>(
        self,
    ) -> FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        U<FRAME_PAYLOAD_SIZE>,
        Sum<Size, U<FRAME_PAYLOAD_SIZE>>,
    >
    where
        Size: Add<U<FRAME_PAYLOAD_SIZE>>,
        Sum<Size, U<FRAME_PAYLOAD_SIZE>>: Unsigned,
        Const<FRAME_PAYLOAD_SIZE>: ToUInt,
        U<FRAME_PAYLOAD_SIZE>: ArrayLength,
        <U<FRAME_PAYLOAD_SIZE> as ArrayLength>::ArrayType<u8>: Copy,
    {
        FrameSpecImpl { frame: PhantomData }
    }

    pub fn with_max_frame_payload_size(
        self,
    ) -> FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        AuxSecHeader,
        HeaderIes,
        PayloadIes,
        Diff<driver::MaxPhyPacketSize<Driver>, Size>,
        driver::MaxPhyPacketSize<Driver>,
    >
    where
        driver::MaxPhyPacketSize<Driver>: Sub<Size>,
        Diff<driver::MaxPhyPacketSize<Driver>, Size>: ArrayLength<ArrayType<u8>: Copy>,
    {
        FrameSpecImpl { frame: PhantomData }
    }
}

#[allow(dead_code)]
impl<
        Driver: RadioDriverConfig + Copy,
        SeqNum: StaticallySized + Copy,
        Addressing: StaticallySized + Copy,
        Security: StaticallySized + Copy,
        HeaderIes: StaticallySized + Copy,
        PayloadIes: StaticallySized + Copy,
        FramePayloadSize: ArrayLength<ArrayType<u8>: Copy>,
        Size: Unsigned,
    >
    FrameSpecImpl<
        Driver,
        SeqNum,
        Addressing,
        Security,
        HeaderIes,
        PayloadIes,
        FramePayloadSize,
        Size,
    >
where
    Self: 'static,
{
    // TODO: Assert that the overall size does not exceed aMaxPhyPacketSize.
    pub fn finalize(self) -> Ppdu<Self> {
        Ppdu::new()
    }

    pub fn from_buffer(self, buffer: &[u8]) -> &Ppdu<Self> {
        Ppdu::from_buffer(buffer)
    }
}

// Whether the IE is a header or payload IE is not relevant in a parsed frame.
// We therefore keep a single enum for all IEs as this allows us to represent
// the parsed IE field with minimal overhead.
#[derive(Clone, Copy)]
pub enum ParsedIe<'frame> {
    // NOTE: We wrap pointers to IEs rather than the parsed IEs themselves to
    //       keep the variant size small as some of the IEs can be quite large.
    //       Variable size IEs need to be wrapped by a "parsed" version of the
    //       IE to allow for typed runtime access.
    // NOTE: There's no need to keep IE meta-data from the frame as it's
    //       content is only relevant while parsing.
    TimeCorrectionIe(&'frame TimeCorrectionIeContent),
    ReducedChannelHoppingIe(&'frame ReducedChannelHoppingIeContent),
    FullChannelHoppingIe(ParsedFullChannelHoppingIeContent<'frame>),
    TschSynchronizationIe(&'frame TschSynchronizationIeContent),
    TschSlotframeAndLinkIe(ParsedTschSlotframeAndLinkIeContent<'frame>),
    ReducedTschTimeslotIe(&'frame ReducedTschTimeslotIeContent),
    FullTschTimeslotIe(&'frame FullTschTimeslotIeContent),
    HeaderTerminationIe1,
    HeaderTerminationIe2,
    PayloadTerminationIe,
    None, // used as terminal or placeholder - the IE preceding this IE is the last IE in the list
} // 12 bytes

pub struct ParsedMpdu<'frame, const MAX_IES: usize> {
    pub fc: FrameControl,                     // 2 bytes
    pub seq_num: Option<u8>,                  // 2 bytes - zero-cost option
    pub addressing: ParsedAddressing<'frame>, // 8 bytes
    #[cfg(feature = "with_security")]
    pub aux_sec_hdr: Option<ParsedAuxSecHeader>, // 20 bytes,
    // A list of pointers to transmuted IE content in the frame.
    // Parsed IE variants will be allocated from a single, statically allocated
    // list. This allows us to keep the overall footprint for parsed IEs
    // minimal.
    pub ies: [ParsedIe<'frame>; MAX_IES], // 12 bytes per IE
    pub frame_payload: &'frame [u8],      // 8 bytes
}

#[cfg(all(target_arch = "arm", feature = "with_security"))]
const _: () = assert!(size_of::<ParsedMpdu<4>>() == 40 + 4 * 12);

#[cfg(all(target_arch = "arm", not(feature = "with_security")))]
const _: () = assert!(size_of::<ParsedMpdu<4>>() == 20 + 4 * 12);

pub struct ParsedPpdu<'frame, const MAX_IES: usize> {
    pub headroom: &'frame [u8],
    pub mpdu: ParsedMpdu<'frame, MAX_IES>,
    pub tailroom: &'frame [u8],
}

#[cfg(all(target_arch = "arm", feature = "with_security"))]
const _: () = assert!(size_of::<ParsedPpdu<4>>() == 56 + 4 * 12);

#[cfg(all(target_arch = "arm", not(feature = "with_security")))]
const _: () = assert!(size_of::<ParsedPpdu<4>>() == 36 + 4 * 12);

#[cfg(test)]
mod test {
    use crate::{
        driver::radio_driver_config,
        fc::{fc_ieee802154_2003, FrameType},
    };

    use super::ieee802154_frame;

    #[test]
    fn test_16bit_fcs() {
        // Uses the 16-bit example from IEEE 802.15.4-2020, section 7.2.11.
        let mut imm_ack = ieee802154_frame(radio_driver_config()).finalize();

        imm_ack.mpdu.mhr.fc = fc_ieee802154_2003(FrameType::Acknowledgment);
        imm_ack.mpdu.mhr.seq_num = 0b0101_0110u8.reverse_bits();
        imm_ack.set_fcs();

        let fcs = imm_ack.mpdu.mfr.fcs;
        assert_eq!(fcs, 0b0010_0111_1001_1110u16.reverse_bits());
    }

    #[test]
    fn test_32bit_fcs() {
        // Uses the 32-bit example from IEEE 802.15.4-2020, section 7.2.11.
        let mut imm_ack = ieee802154_frame(radio_driver_config().with_4byte_fcs()).finalize();

        imm_ack.mpdu.mhr.fc = fc_ieee802154_2003(FrameType::Acknowledgment);
        imm_ack.mpdu.mhr.seq_num = 0b0101_0110u8.reverse_bits();
        imm_ack.set_fcs();

        let fcs = imm_ack.mpdu.mfr.fcs;
        assert_eq!(
            fcs,
            0b0101_1101_0010_1001_1111_1010_0010_1000u32.reverse_bits()
        );
    }
}
