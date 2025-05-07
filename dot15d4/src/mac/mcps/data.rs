use crate::{mac::MacService, upper::UpperLayer};
use dot15d4_frame3::driver::{Rx, Tx};
use dot15d4_frame3::mpdu::MpduFrame;
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

pub enum DataError {
    // TODO: not supported
    TransactionOverflow,
    // TODO: not supported
    TransactionExpired,
    // TODO: not supported
    ChannelAccessFailure,
    // TODO: not supported
    InvalidAddress,
    // TODO: not supported
    NoAck,
    // TODO: not supported
    CounterError,
    // TODO: not supported
    FrameTooLong,
    // TODO: not supported
    InvalidParameter,
}

pub struct DataRequest<'mpdu> {
    pub mpdu: MpduFrame<'mpdu, Tx>,
}

pub struct DataConfirm {
    /// Timestamp of frame transmission
    pub timestamp: u32,
    /// Whether the frame has been acknowledged or not
    pub acked: bool,
}

pub struct DataIndication<'mpdu> {
    /// buffer containing the received frame payload
    pub mpdu: MpduFrame<'mpdu, Rx>,
    /// Timestamp of frame reception
    pub timestamp: u32,
}

impl<'svc, Rng, U, TIMER> MacService<'svc, Rng, U, TIMER>
where
    Rng: RngCore,
    U: UpperLayer,
    TIMER: DelayNs + Clone,
{
    /// Requests the transfer of data to another device
    pub async fn mcps_data_request(
        &self,
        mut mpdu: MpduFrame<'svc, Tx>,
    ) -> Result<DataConfirm, DataError> {
        let sequence_number = Self::set_ack(&mut mpdu);

        self.phy_send(mpdu.driver_frame()).await;
        let acked = match sequence_number {
            Some(sequence_number) => self.wait_for_ack(sequence_number).await,
            _ => true,
        };
        Ok(DataConfirm {
            // TODO: support timestamp
            timestamp: 0,
            acked,
        })
    }

    pub async fn mcps_data_indication(&self, indication: DataIndication<'svc>) {
        self.upper_layer
            .process_mac_indication(crate::mac::primitives::MacIndication::McpsData(indication))
            .await;
    }
}
