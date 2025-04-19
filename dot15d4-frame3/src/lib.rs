#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(feature = "strict", deny(warnings))]
#![allow(dead_code)]

pub mod addressing;
use core::future::Future;

pub use addressing::*;

pub mod driver;
pub mod driver_nrf;
pub mod frame_control;
#[cfg(target_arch = "arm")]
mod heapless_bufs;
pub mod ie;
pub mod mpdu;
pub mod sec;

/// An error that can occur when reading or writing an IEEE 802.15.4 frame.
#[derive(Debug, Clone, Copy)]
pub struct Error;

/// A type alias for `Result<T, frame::Error>`.
pub type Result<T> = core::result::Result<T, Error>;

pub trait FrameToken: Sized {
    type Repr;

    /// Acquires and guarantees all resources required to consume the token,
    /// i.e. send/or receive a frame of the type and size specified by the given
    /// frame representation.
    ///
    /// MAY block to exert backpressure onto the caller.
    fn acquire(repr: Self::Repr, sdu_length: usize) -> impl Future<Output = Result<Self>>;
}

pub trait TokenToFrame<'buffer>: Sized {
    type Pdu: ?Sized + 'buffer;

    /// Adds a slice referencing the buffer to the frame token. The buffer SHALL
    /// be at least as large as the size of the frame represented by
    /// [`Self::Repr`].
    ///
    /// May be called by any sublayer during the acquisition phase, once the
    /// exact size of the requested buffer is known.
    fn set_buffer(
        &mut self,
        buffer: &'buffer mut [u8],
    ) -> Result<impl Frame<'buffer, Pdu = Self::Pdu>>;
}

pub trait Frame<'buffer>: Sized {
    type Pdu: ?Sized;

    /// Consumes the frame and returns the underlying raw buffer (including
    /// headroom/tailroom).
    fn buffer(self) -> &'buffer mut [u8];

    /// Wraps relevant parts of the next lower-layer's SDU in a layer-specific
    /// frame reader, so that this layer's protocol header and footer can be
    /// read.
    fn pdu_ref(&self) -> &Self::Pdu;

    /// Wraps relevant parts of the next lower-layer's SDU in a layer-specific
    /// frame writer, so that this layer's protocol header and footer can be
    /// written to.
    fn pdu_mut(&mut self) -> &mut Self::Pdu;

    /// Retrieves the part of the buffer reserved for the SDU.
    fn sdu_ref(&self) -> &[u8];

    /// Retrieves the part of the buffer reserved for the SDU for writing.
    fn sdu_mut(&mut self) -> &mut [u8];
}

#[cfg(test)]
mod test {
    use typenum::Unsigned;

    use super::*;

    #[cfg(feature = "tsch")]
    use crate::ie::Ie;
    #[cfg(feature = "security")]
    use crate::sec::{Security, SecurityLevel};
    use crate::{
        addressing::{Address, Addressing, PanIdCompression},
        driver::{DriverConfig, DriverFrameRepr},
        driver_nrf::{DRIVER_OVERHEAD, FCS_LEN},
        frame_control::{FrameType, FrameVersion},
        mpdu::{imm_ack_frame, MpduBuilder, SeqNr, IMM_ACK_BUF_LEN, IMM_ACK_FRAME_REPR},
    };

    #[test]
    fn test_frame_builder_api_and_size() {
        const PAN_ID: u16 = 0x1234;

        let mpdu = MpduBuilder::new();

        #[cfg(feature = "security")]
        let mpdu = mpdu.with_security(Security::Source4Byte(SecurityLevel::EncMic32));

        #[cfg(not(feature = "security"))]
        let mpdu = mpdu.without_security();

        let mpdu = mpdu.with_frame_config(
            false,
            SeqNr::Yes,
            Addressing::new(
                Address::Short(PAN_ID),
                Address::Short(PAN_ID),
                PanIdCompression::Yes,
            ),
        );

        #[cfg(feature = "tsch")]
        let slotframes = [2, 3, 4];

        #[cfg(feature = "tsch")]
        let ies = [
            Ie::TimeCorrectionHeaderIe,
            Ie::FullTschTimeslotNestedIe,
            Ie::TschSlotframeAndLinkNestedIe(&slotframes),
        ];

        #[cfg(feature = "tsch")]
        let mpdu = mpdu.with_ies(&ies);

        #[cfg(not(feature = "tsch"))]
        let mpdu = mpdu.without_ies();

        #[cfg(all(not(feature = "tsch"), not(feature = "security")))]
        assert_eq!(size_of_val(&mpdu), 12);

        #[cfg(all(feature = "security", not(feature = "tsch")))]
        assert_eq!(size_of_val(&mpdu), 14);

        #[cfg(all(feature = "tsch", not(feature = "security")))]
        assert_eq!(size_of_val(&mpdu), 32);

        #[cfg(all(feature = "security", feature = "tsch"))]
        assert_eq!(size_of_val(&mpdu), 32);
    }

    const TEST_SEQ_NUM: u8 = 55;

    #[test]
    fn test_imm_ack_frame() {
        const IMM_ACK_LEN: usize = 3;

        let mut buffer = [0; IMM_ACK_BUF_LEN];
        let frame = imm_ack_frame(TEST_SEQ_NUM, &mut buffer);

        assert_eq!(IMM_ACK_BUF_LEN, DRIVER_OVERHEAD + IMM_ACK_LEN + FCS_LEN);
        assert_eq!(
            size_of_val(&frame),
            round_to_alignment(
                size_of_val(&IMM_ACK_FRAME_REPR) + IMM_ACK_BUF_LEN,
                align_of_val(&frame)
            )
        );

        let expected_buffer = vec![
            0,
            FrameType::Ack as u8,
            (FrameVersion::Ieee802154_2006 as u8) << 4,
            TEST_SEQ_NUM,
            0,
            0,
        ];
        assert_eq!(frame.as_bytes(), &expected_buffer);
    }

    fn round_to_alignment(size: usize, alignment: usize) -> usize {
        assert!(alignment > 0 && ((alignment & (alignment - 1)) == 0));

        let size = size as isize;
        let alignment = alignment as isize;

        return ((size + alignment - 1) & -alignment) as usize;
    }
}
