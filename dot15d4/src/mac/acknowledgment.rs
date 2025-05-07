use crate::{
    mac::{
        constants::{MAC_AIFS_PERIOD, MAC_SIFS_PERIOD},
        MacService,
    },
    sync::{select, Either},
    time::Duration,
    upper::UpperLayer,
};
use dot15d4_frame3::{
    driver::{Rx, Tx},
    frame_control::FrameType,
    mpdu::{imm_ack_frame, MpduFrame},
};
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

impl<'svc, Rng, U, TIMER> MacService<'svc, Rng, U, TIMER>
where
    Rng: RngCore,
    U: UpperLayer,
    TIMER: DelayNs + Clone,
{
    /// Transmit acknowledgment for a frame that has been received.
    /// Returns when acknowledgment has been transmitted by the radio.
    ///
    /// * `ack_mpdu` - Frame with acknowledgment
    pub(crate) async fn transmit_ack(&self, ack_mpdu: Option<MpduFrame<'tx, Tx>>) {
        if let Some(ack_frame) = ack_mpdu {
            self.phy_send(ack_frame.driver_frame()).await;
        }
    }

    /// Prepare an acknowledgment frame if one has to be sent for the given
    /// received frame.
    ///
    /// * `rx_mpdu` - Received frame to potentially acknowledge
    /// * `buffer` - Mutable reference to a buffer that will store the
    ///    potential ack frame to generate.
    pub(crate) fn prepare_ack<'buf>(
        &self,
        rx_mpdu: &MpduFrame<'rx, Rx>,
        buffer: &'tx mut [u8],
    ) -> Option<MpduFrame<'tx, Tx>> {
        if !rx_mpdu.frame_control().ack_request() {
            return None;
        }

        let seq_num = rx_mpdu.sequence_number();
        if seq_num.is_none() {
            // TODO: This is an invalid frame which should be logged.
            return None;
        }

        Some(imm_ack_frame(seq_num.unwrap(), buffer))
    }

    /// Wait for the reception of an acknowledgment for a specific sequence
    /// number. Time out if ack is not received within a specific delay.
    /// Return `true` if such an ack is received, return `else` otherwise (or
    /// if timed out).
    ///
    /// * `sequence_number` - Sequence number of the frame waiting for ack
    pub(crate) async fn wait_for_ack(&self, sequence_number: u8) -> bool {
        let mut timer = self.timer.clone();
        // We expect an ACK to come back AIFS + time for an ACK to travel + SIFS (guard)
        // An ACK is 3 bytes + 6 bytes (PHY header) long
        // and should take around 288us at 250kbps to get back
        let delay = MAC_AIFS_PERIOD + MAC_SIFS_PERIOD + Duration::from_us(288);

        match select::select(
            async {
                // We may receive multiple frames during that period of time.
                // non-matching frames are dropped.
                // TODO: Non-matching frames should be handled normally, as the
                //       ACK could simply have been lost and we're now dropping
                //       legit frames from other devices until the timeout fires.
                loop {
                    let ack_frame = self.phy_receive().await;
                    let ack_mpdu = MpduFrame::parse(ack_frame);

                    if !matches!(ack_mpdu.frame_control().frame_type(), FrameType::Ack) {
                        continue;
                    }

                    let seq_num = ack_mpdu.sequence_number();
                    if seq_num.is_none() {
                        // TODO: log invalid frame
                        continue;
                    }

                    if sequence_number == seq_num.unwrap() {
                        break;
                    }
                }
            },
            // Timeout for waiting on an ACK
            async {
                timer.delay_us(delay.as_us() as u32).await;
                info!("Expired !");
            },
        )
        .await
        {
            Either::First(_) => true,
            Either::Second(_) => false,
        }
    }

    /// Check if the given frame needs to be acknowledged, based on current
    /// buffer content and frame addressing. If so, acknowledgment request is
    /// set in the frame.
    ///
    /// * `frame` - Frame buffer to check and update, if necessary.
    pub(crate) fn set_ack(mpdu: &mut MpduFrame<'svc, Tx>) -> Option<u8> {
        match mpdu.addressing().and_then(|addr| addr.dst_address()) {
            Some(addr) if addr.is_unicast() => {
                mpdu.frame_control().set_ack_request(true);
                mpdu.sequence_number()
            }
            _ => None,
        }
    }
}
