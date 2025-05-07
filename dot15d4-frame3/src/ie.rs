use const_for::const_for;
use core::marker::PhantomData;

use crate::mpdu::{MacFrameConfigComplete, MpduBuilder};

#[derive(Clone, Copy, Debug)]
pub enum Ie<'ie> {
    TimeCorrectionHeaderIe,
    ReducedChannelHoppingNestedIe,
    FullChannelHoppingNestedIe(u8, u8), // num channels, ext bitmap length (in bytes)
    TschSynchronizationNestedIe,
    TschSlotframeAndLinkNestedIe(&'ie [u8]), // for each slotframe descriptor: number of links
    ReducedTschTimeslotNestedIe,
    FullTschTimeslotNestedIe,
} // TSCH: 12 bytes, else 0 bytes

impl Ie<'_> {
    // returns (header_ie_len, nested_ie_len)
    pub const fn length(&self) -> (usize, usize) {
        if cfg!(feature = "tsch") {
            const IE_HDR_SIZE: usize = 2;

            let (header_ie_content_len, nested_ie_content_len) = match self {
                Ie::TimeCorrectionHeaderIe => (2, 0),
                Ie::ReducedChannelHoppingNestedIe => (0, 1),
                Ie::FullChannelHoppingNestedIe(num_channels, ext_bm_len) => {
                    (0, 12 + (*num_channels as usize) + (*ext_bm_len as usize))
                }
                Ie::TschSynchronizationNestedIe => (0, 6),
                Ie::TschSlotframeAndLinkNestedIe(slotframes) => {
                    const LINK_INFO_LEN: usize = 5;
                    const SLOTFRAME_DESCRIPTOR_HDR_LEN: usize = 4;
                    const TSCH_SLOTFRAME_AND_LINK_HDR_LEN: usize = 1;
                    let mut content_len = TSCH_SLOTFRAME_AND_LINK_HDR_LEN
                        + slotframes.len() * SLOTFRAME_DESCRIPTOR_HDR_LEN;
                    const_for!(sf_idx in 0..slotframes.len() => {
                        let link_info_fields = slotframes[sf_idx];
                        content_len += (link_info_fields as usize) * LINK_INFO_LEN;

                    });
                    (0, content_len)
                }
                Ie::ReducedTschTimeslotNestedIe => (0, 1),
                Ie::FullTschTimeslotNestedIe => (0, 25),
            };

            if header_ie_content_len > 0 {
                (IE_HDR_SIZE + header_ie_content_len, 0)
            } else if nested_ie_content_len > 0 {
                (0, IE_HDR_SIZE + nested_ie_content_len)
            } else {
                unreachable!()
            }
        } else {
            (0, 0)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct IeList<'ies>(pub &'ies [Ie<'ies>]);

impl IeList<'_> {
    pub const fn length(&self, has_frame_payload: bool) -> usize {
        const IE_HDR_SIZE: usize = 2;

        let mut len = 0;
        let mut has_header_ie = false;
        let mut has_nested_ie = false;

        const_for!(ie_idx in 0..self.0.len() => {
            let ie = self.0[ie_idx];
            let (header_ie_len, nested_ie_len) = ie.length();

            if header_ie_len > 0 {
                has_header_ie = true;
                len += header_ie_len;
            } else if nested_ie_len > 0 {
                has_nested_ie = true;
                len += nested_ie_len;
            } else {
                unreachable!()
            }
        });

        if has_nested_ie {
            // MLME IE
            len += IE_HDR_SIZE;
        }

        // See IEEE 802.15.4-2020, section 7.4.1
        len += match (has_header_ie, has_nested_ie, has_frame_payload) {
            // Header Termination | Payload Termination
            // ========================================
            // None               | None
            (false, false, false) | (true, false, false) | (false, false, true) => 0,
            // HT1                | None (Optional)
            (false, true, false) | (true, true, false) |
            // HT2                | None
            (true, false, true) => IE_HDR_SIZE,
            // HT1                | PT
            (false, true, true) | (true, true, true) => 2*IE_HDR_SIZE,
        };

        len
    }
}

#[derive(Debug)]
pub struct MacIeConfig;

impl<'frame> MpduBuilder<'frame, MacIeConfig> {
    #[cfg(feature = "tsch")]
    pub const fn with_ies(self, ies: &'frame [Ie]) -> MpduBuilder<'frame, MacFrameConfigComplete> {
        MpduBuilder {
            tsch_mode: self.tsch_mode,
            seq_nr: self.seq_nr,
            addressing: self.addressing,
            #[cfg(feature = "security")]
            security: self.security,
            ies: Some(IeList(ies)),
            stage: PhantomData,
        }
    }

    pub const fn without_ies(self) -> MpduBuilder<'frame, MacFrameConfigComplete> {
        MpduBuilder {
            tsch_mode: self.tsch_mode,
            seq_nr: self.seq_nr,
            addressing: self.addressing,
            #[cfg(feature = "security")]
            security: self.security,
            #[cfg(feature = "tsch")]
            ies: None,
            stage: PhantomData,
        }
    }
}
