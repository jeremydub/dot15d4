#![no_std]

use core::{
    ops::{Deref, DerefMut},
    usize,
};

use addr::parse_addressing;
use byteorder::{ByteOrder, LE};
use driver::RadioDriverConfig;
use fc::FrameControl;
use frame::{ParsedIe, ParsedMpdu, ParsedPpdu};
use generic_array::ArrayLength;
use header_ie::parse_header_ie;
use payload_ie::{parse_nested_ie, parse_payload_ie, ParsedPayloadIe};
#[cfg(feature = "with_security")]
use sec::parse_aux_sec_header;
use typenum::{Unsigned, U0, U1, U2, U4};

pub mod addr;
pub mod command;
pub mod driver;
pub mod fc;
pub mod frame;
pub mod header_ie;
pub mod payload_ie;
pub mod pib;
pub mod sec;

pub trait StaticallySized {
    type Size: Unsigned;
}

impl StaticallySized for () {
    type Size = U0;
}

impl StaticallySized for u8 {
    type Size = U1;
}

impl StaticallySized for u16 {
    type Size = U2;
}

impl StaticallySized for u32 {
    type Size = U4;
}

pub type SizeOf<Type> = <Type as StaticallySized>::Size;

#[derive(Clone, Copy)]
pub struct GenericByteBuffer<N: ArrayLength<ArrayType<u8>: Copy>> {
    buffer: generic_array::GenericArray<u8, N>,
}

impl<N: ArrayLength<ArrayType<u8>: Copy>> Deref for GenericByteBuffer<N> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.buffer.as_slice()
    }
}

impl<N: ArrayLength<ArrayType<u8>: Copy>> DerefMut for GenericByteBuffer<N> {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.buffer.as_mut_slice()
    }
}

// TODO: check for remaining buffer length everywhere.
// TODO: correctly convert all bit fields from/to little endian
// TODO: write endianness-safe setters/getters for all structs and make fields private
// TODO: migrate all content and checks over from the existing frame implementation
pub fn parse<DriverConfig: RadioDriverConfig, const MAX_IES: usize>(
    buffer: &[u8],
    driver_config: DriverConfig,
) -> Result<ParsedPpdu<MAX_IES>, ()> {
    let headroom_size = <driver::Headroom<DriverConfig> as Unsigned>::to_usize();
    let max_phy_packet_size = <driver::MaxPhyPacketSize<DriverConfig> as Unsigned>::to_usize();
    let tailroom_size = <driver::Tailroom<DriverConfig> as Unsigned>::to_usize();
    // Ensure that the buffer has the correct size.
    assert_eq!(
        buffer.len(),
        headroom_size + max_phy_packet_size + tailroom_size
    );

    let headroom = &buffer[..headroom_size];
    let (buffer, mpdu) = parse_mpdu(
        &buffer[headroom_size..(headroom_size + max_phy_packet_size)],
        driver_config,
    )?;
    let tailroom = &buffer[..tailroom_size];

    Ok(ParsedPpdu {
        headroom,
        mpdu,
        tailroom,
    })
}

// This function must be called with a buffer that only contains the MPDU, i.e.
// driver headroom and tailroom has already been split off.
fn parse_mpdu<DriverConfig: RadioDriverConfig, const MAX_IES: usize>(
    buffer: &[u8],
    _driver_config: DriverConfig,
) -> Result<(&[u8], ParsedMpdu<MAX_IES>), ()> {
    let mfr_len = <driver::FcsSize<DriverConfig> as Unsigned>::to_usize();
    let buffer = &buffer[..(buffer.len() - mfr_len)];

    let fc = FrameControl::from_bits(LE::read_u16(buffer));
    let buffer = &buffer[2..];

    let (buffer, seq_num) = if fc.seq_num_suppr() {
        (buffer, None)
    } else {
        (&buffer[1..], Some(buffer[0]))
    };

    let (buffer, addressing) = parse_addressing(fc, buffer)?;

    #[cfg(feature = "with_security")]
    let (buffer, aux_sec_hdr) = if fc.security_enabled() {
        let (buffer, aux_sec_hdr) = parse_aux_sec_header(buffer)?;
        (buffer, Some(aux_sec_hdr))
    } else {
        (buffer, None)
    };

    let (buffer, ies) = if fc.ie_present() {
        parse_ies(buffer)?
    } else {
        (buffer, [ParsedIe::None; MAX_IES])
    };

    let frame_payload = buffer;

    Ok((
        buffer,
        ParsedMpdu {
            fc,
            seq_num,
            addressing,
            #[cfg(feature = "with_security")]
            aux_sec_hdr,
            ies,
            frame_payload,
        },
    ))
}

// This method SHALL only be called when the frame contains IEs (i.e. the frame
// control field has the ie_present flag set).
//
// The buffer is expected to point at the start of the IE field and to not
// contain the MAC footer (FCS).
//
// Termination IEs will not be included in the result.
pub fn parse_ies<const MAX_IES: usize>(buffer: &[u8]) -> Result<(&[u8], [ParsedIe; MAX_IES]), ()> {
    let mut parsed_ies: [ParsedIe; MAX_IES] = [ParsedIe::None; MAX_IES];
    let mut num_ies: usize = 0;

    // Loop until either the buffer has been fully parsed or we find a header
    // termination IE.
    let has_payload_ies = loop {
        let (buffer, header_ie) = parse_header_ie(buffer)?;
        match header_ie {
            ParsedIe::HeaderTerminationIe1 => break true,
            ParsedIe::HeaderTerminationIe2 => break false,
            _ => {}
        }

        if num_ies >= MAX_IES {
            return Err(());
        }
        parsed_ies[num_ies] = header_ie;
        num_ies += 1;

        if buffer.len() == 0 {
            break false;
        }
    };

    if has_payload_ies {
        // Loop until either the buffer has been fully parsed or we find a payload
        // termination IE.
        loop {
            let (buffer, payload_ie) = parse_payload_ie(buffer)?;
            let mut nested_ies = match payload_ie {
                ParsedPayloadIe::MlmeIe(nested_ies) => nested_ies,
                ParsedPayloadIe::PayloadTerminationIe => break,
            };

            if num_ies >= MAX_IES {
                return Err(());
            }

            while nested_ies.len() > 0 {
                let (remaining_nested_ies, nested_ie) = parse_nested_ie(nested_ies)?;
                nested_ies = remaining_nested_ies;

                parsed_ies[num_ies] = nested_ie;
                num_ies += 1;
            }

            if buffer.len() == 0 {
                break;
            }
        }
    }

    Ok((buffer, parsed_ies))
}

#[cfg(test)]
mod test {
    extern crate std;

    use std::vec;

    use crate::addr::addressing;
    use crate::driver::radio_driver_config;
    use crate::fc::{FrameType, FrameVersion};
    use crate::frame::ieee802154_frame;
    use crate::header_ie::{
        header_ies, header_termination_ie_1, time_correction_ie, TimeCorrectionIeContent,
    };
    use crate::payload_ie::{
        full_channel_hopping_ie, mlme_ie, payload_ies, payload_termination_ie,
        reduced_tsch_timeslot_ie, tsch_slotframe_and_link_ie,
    };
    use crate::sec::aux_sec_hdr;

    use super::*;

    #[test]
    fn test_packed_alignment() {
        // Fabricate a 1-aligned slice to simulate access to unaligned fields or
        // 1-packed structures in a frame buffer.
        #[repr(C, align(2))]
        struct AlignedBufferHolder {
            aligned_buffer: [u8; 3],
        }
        let mut aligned_buffer_holder = AlignedBufferHolder {
            aligned_buffer: [0; 3],
        };
        let aligned_slice = aligned_buffer_holder.aligned_buffer.as_mut_slice();
        let unaligned_slice = &mut aligned_slice[1..=2];

        // Prove that the fabricated slice is 1-aligned.
        let (pre, mid, post) = unsafe { unaligned_slice.align_to::<u16>() };
        assert_eq!(pre.len(), 1);
        assert_eq!(mid.len(), 0);
        assert_eq!(post.len(), 1);

        // Represent 0x1234 in little endian.
        unaligned_slice[0] = 0x34;
        unaligned_slice[1] = 0x12;

        // Prove that the produced u16 contains the expected value.
        // NOTE: Requires conversion to the host's endianness.
        let expected_value = u16::from_le_bytes([unaligned_slice[0], unaligned_slice[1]]);
        assert_eq!(expected_value, 0x1234);

        // Transmute the unaligned buffer to a 1-packed struct.
        let ie: &TimeCorrectionIeContent = bytemuck::from_bytes(unaligned_slice);

        // Force unaligned access.
        // NOTE: The struct has been transmuted from a little endian byte array
        //       without endianness conversion so it must be little endian
        //       encoded on any platform.
        let time_sync_info_le = ie.time_sync_info;
        assert_eq!(time_sync_info_le, expected_value.to_le());
    }

    #[test]
    fn test_structure_and_size() {
        const DRIVER_HEADROOM: usize = 2;
        const DRIVER_TAILROOM: usize = 1;

        let driver_config = radio_driver_config()
            .with_headroom::<DRIVER_HEADROOM>()
            .with_tailroom::<DRIVER_TAILROOM>();

        const DRIVER_OVERHEAD: usize = DRIVER_HEADROOM + DRIVER_TAILROOM;

        const FC_LEN: usize = 2;

        const SEQ_LEN: usize = 1;

        let addressing = addressing()
            .with_dst_pan_id()
            .with_short_dst_addr()
            .with_extended_src_addr()
            .finalize();

        const PAN_ID_LEN: usize = 2;
        const SHORT_ADDR_LEN: usize = 2;
        const EXT_ADDR_LEN: usize = 8;
        const ADDR_LEN: usize = PAN_ID_LEN + SHORT_ADDR_LEN + EXT_ADDR_LEN;

        let frame_security = aux_sec_hdr()
            .with_frame_counter()
            .with_key_id()
            .with_4byte_source()
            .finalize();

        const SC_LEN: usize = 1;
        const FRAME_COUNTER_LEN: usize = 4;
        const KEY_ID_LEN: usize = 5;
        const AUX_SEC_HDR_LEN: usize = SC_LEN + FRAME_COUNTER_LEN + KEY_ID_LEN;

        let header_ies = header_ies()
            .add_header_ie(time_correction_ie())
            .add_header_ie(header_termination_ie_1())
            .finalize();

        const HEADER_IE_HDR_LEN: usize = 2;
        const TIME_CORRECTION_IE_CONTENT_LEN: usize = 2;
        const HEADER_TERMINATION_IE_1_CONTENT_LEN: usize = 0;
        const HEADER_IES_LEN: usize = 2 * HEADER_IE_HDR_LEN
            + TIME_CORRECTION_IE_CONTENT_LEN
            + HEADER_TERMINATION_IE_1_CONTENT_LEN;

        let channel_hopping_ie = full_channel_hopping_ie()
            .with_ext_bm_len::<2>()
            .with_hopping_seq_len::<5>()
            .finalize();

        const NESTED_IE_HDR_LEN: usize = 2;
        const CHANNEL_HOPPING_HDR_LEN: usize = 12;
        const EXT_BM_LEN: usize = 2;
        const HOPPING_SEQ_LEN: usize = 5;
        const CHANNEL_HOPPING_IE_LEN: usize =
            NESTED_IE_HDR_LEN + CHANNEL_HOPPING_HDR_LEN + EXT_BM_LEN + HOPPING_SEQ_LEN;

        let tsch_sf_and_link_ie = tsch_slotframe_and_link_ie()
            .add_slotframe_descriptor()
            .with_num_links::<2>()
            .add_slotframe_descriptor()
            .with_num_links::<3>()
            .add_slotframe_descriptor()
            .with_num_links::<4>()
            .finalize();

        const LINK_INFO_LEN: usize = 5;
        const SLOTFRAME_DESCRIPTOR_HDR_LEN: usize = 4;
        const TSCH_SLOTFRAME_AND_LINK_HDR_LEN: usize = 1;
        const TSCH_SLOTFRAME_AND_LINK_LEN: usize =
            // TSCH Slotframe and Link IE
            NESTED_IE_HDR_LEN + TSCH_SLOTFRAME_AND_LINK_HDR_LEN
            // Slotframe Descriptors
            + 3 * SLOTFRAME_DESCRIPTOR_HDR_LEN
            // Link Information fields
            + (2+3+4) * LINK_INFO_LEN;

        const TSCH_TIMESLOT_IE_LEN: usize = NESTED_IE_HDR_LEN + 1;

        let mlme_ie = mlme_ie()
            .add_long_nested_ie(channel_hopping_ie)
            .add_short_nested_ie(tsch_sf_and_link_ie)
            .add_short_nested_ie(reduced_tsch_timeslot_ie())
            .finalize();

        const PAYLOAD_IE_HDR_LEN: usize = 2;
        const MLME_IE_LEN: usize = PAYLOAD_IE_HDR_LEN
            + CHANNEL_HOPPING_IE_LEN
            + TSCH_TIMESLOT_IE_LEN
            + TSCH_SLOTFRAME_AND_LINK_LEN;

        let payload_ies = payload_ies()
            .add_payload_ie(mlme_ie)
            .add_payload_ie(payload_termination_ie())
            .finalize();

        const PAYLOAD_TERMINATION_IE_LEN: usize = PAYLOAD_IE_HDR_LEN;
        const PAYLOAD_IES_LEN: usize = MLME_IE_LEN + PAYLOAD_TERMINATION_IE_LEN;

        const FRAME_PAYLOAD_LEN: usize = 3;

        let basic_ppdu = ieee802154_frame(driver_config)
            .with_addressing(addressing)
            .with_aux_sec_header(frame_security)
            .with_header_ies(header_ies)
            .with_payload_ies(payload_ies);

        let ppdu_with_seq_num = basic_ppdu
            .with_frame_payload_size::<FRAME_PAYLOAD_LEN>()
            .finalize();

        const MHR_LEN_WO_SEQ: usize = FC_LEN + ADDR_LEN + AUX_SEC_HDR_LEN + HEADER_IES_LEN;

        const MHR_LEN: usize = MHR_LEN_WO_SEQ + SEQ_LEN;

        let fc = ppdu_with_seq_num.mpdu.mhr.fc;
        assert_eq!(size_of_val(&fc), FC_LEN);
        assert_eq!(size_of_val(&ppdu_with_seq_num.mpdu.mhr.seq_num), SEQ_LEN);
        assert_eq!(
            size_of_val(&ppdu_with_seq_num.mpdu.mhr.addressing),
            ADDR_LEN
        );
        assert_eq!(
            size_of_val(&ppdu_with_seq_num.mpdu.mhr.aux_sec_hdr),
            AUX_SEC_HDR_LEN
        );
        assert_eq!(
            size_of_val(&ppdu_with_seq_num.mpdu.mhr.header_ies),
            HEADER_IES_LEN
        );
        assert_eq!(size_of_val(&ppdu_with_seq_num.mpdu.mhr), MHR_LEN);

        let ppdu_without_seq_num = basic_ppdu
            .without_seq_num()
            .with_max_frame_payload_size()
            .finalize();

        assert_eq!(size_of_val(&ppdu_without_seq_num.mpdu.mhr), MHR_LEN_WO_SEQ);

        let payload_ies = ppdu_with_seq_num.mpdu.mac_payload.payload_ies;

        let mlme_ie = payload_ies.prev.payload_ie;
        let mlme_ie_content = mlme_ie.content;

        // TSCH Timeslot IE
        let tsch_timeslot_ie = mlme_ie_content.nested_ie;
        assert_eq!(size_of_val(&tsch_timeslot_ie), TSCH_TIMESLOT_IE_LEN);

        // TSCH Slotframe And Link IE
        let tsch_slotframe_and_link_ie = mlme_ie_content.prev.nested_ie;
        let tsch_slotframe_and_link_ie_content = tsch_slotframe_and_link_ie.content;
        let slotframe_descriptor_1 = tsch_slotframe_and_link_ie_content
            .slotframe_descriptors
            .prev
            .prev
            .slotframe_descriptor;
        assert_eq!(
            size_of_val(&slotframe_descriptor_1),
            SLOTFRAME_DESCRIPTOR_HDR_LEN + 2 * LINK_INFO_LEN
        );
        assert_eq!(
            size_of_val(&tsch_slotframe_and_link_ie),
            TSCH_SLOTFRAME_AND_LINK_LEN
        );
        let slotframe_descriptor_2 = tsch_slotframe_and_link_ie_content
            .slotframe_descriptors
            .prev
            .slotframe_descriptor;
        assert_eq!(
            size_of_val(&slotframe_descriptor_2),
            SLOTFRAME_DESCRIPTOR_HDR_LEN + 3 * LINK_INFO_LEN
        );
        let slotframe_descriptor_3 = tsch_slotframe_and_link_ie_content
            .slotframe_descriptors
            .slotframe_descriptor;
        assert_eq!(
            size_of_val(&slotframe_descriptor_3),
            SLOTFRAME_DESCRIPTOR_HDR_LEN + 4 * LINK_INFO_LEN
        );

        // Channel Hopping IE
        let channel_hopping_ie = mlme_ie_content.prev.prev.nested_ie;
        assert_eq!(size_of_val(&channel_hopping_ie), CHANNEL_HOPPING_IE_LEN);

        assert_eq!(size_of_val(&mlme_ie), MLME_IE_LEN);

        // Payload Termination IE
        let payload_termination_ie = payload_ies.payload_ie;
        assert_eq!(
            size_of_val(&payload_termination_ie),
            PAYLOAD_TERMINATION_IE_LEN
        );

        // Payload IE List
        assert_eq!(size_of_val(&payload_ies), PAYLOAD_IES_LEN);

        // Manually sized Frame Payload
        assert_eq!(
            size_of_val(&ppdu_with_seq_num.mpdu.mac_payload.frame_payload),
            FRAME_PAYLOAD_LEN
        );

        // Mac Payload
        const MAC_PAYLOAD_LEN: usize = PAYLOAD_IES_LEN + FRAME_PAYLOAD_LEN;
        assert_eq!(
            size_of_val(&ppdu_with_seq_num.mpdu.mac_payload),
            MAC_PAYLOAD_LEN
        );

        const MFR_LEN: usize = 2;

        // Mac Footer
        assert_eq!(size_of_val(&ppdu_with_seq_num.mpdu.mfr), MFR_LEN);

        // MPDU
        const MPDU_LEN: usize = MHR_LEN + MAC_PAYLOAD_LEN + MFR_LEN;
        assert_eq!(size_of_val(&ppdu_with_seq_num.mpdu), MPDU_LEN);

        // Driver-specific head-/tailroom.
        assert_eq!(size_of_val(&ppdu_without_seq_num.headroom), DRIVER_HEADROOM);
        assert_eq!(size_of_val(&ppdu_without_seq_num.tailroom), DRIVER_TAILROOM);

        // PPDU
        assert_eq!(size_of_val(&ppdu_with_seq_num), MPDU_LEN + DRIVER_OVERHEAD);

        // PPDU with max frame payload
        const MAX_PHY_PKT_SIZE: usize = 127;

        assert_eq!(
            size_of_val(&ppdu_without_seq_num.mpdu.mac_payload.frame_payload),
            MAX_PHY_PKT_SIZE - MHR_LEN_WO_SEQ - PAYLOAD_IES_LEN - MFR_LEN
        );

        // Mac Payload
        assert_eq!(
            size_of_val(&ppdu_without_seq_num.mpdu.mac_payload),
            MAX_PHY_PKT_SIZE - MHR_LEN_WO_SEQ - MFR_LEN
        );

        // MPDU
        assert_eq!(size_of_val(&ppdu_without_seq_num.mpdu), MAX_PHY_PKT_SIZE);

        // Initialization
        assert_eq!([0; MAX_PHY_PKT_SIZE], ppdu_without_seq_num.mpdu.as_bytes());
        assert_eq!(
            [0; MAX_PHY_PKT_SIZE + DRIVER_OVERHEAD],
            ppdu_without_seq_num.as_bytes()
        );
    }

    #[test]
    fn test_imm_ack() {
        let driver_config = radio_driver_config();

        const TEST_SEQ_NUM: u8 = 55;
        let imm_ack = ieee802154_frame(driver_config).imm_ack(TEST_SEQ_NUM);

        assert_eq!(size_of_val(&imm_ack), 5);

        let mut expected_mpdu = vec![
            FrameType::Acknowledgment as u8,
            (FrameVersion::Ieee802154_2006 as u8) << 4,
            TEST_SEQ_NUM,
        ];

        let expected_mfr = imm_ack
            .mpdu
            .mfr
            .get_crc_impl()
            .checksum(&expected_mpdu)
            .to_le_bytes();
        expected_mpdu.extend_from_slice(&expected_mfr);

        assert_eq!(&expected_mpdu, imm_ack.as_bytes());
    }
}
