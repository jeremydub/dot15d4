use crate::upper::UpperLayer;
use dot15d4_frame3::driver::DriverConfig;
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

use super::MacService;

pub enum SetError {
    InvalidParameter,
}

/// Attributes that may be written by an upper layer
pub enum SetRequestAttribute {
    // IEEE 802.15.4-2020, section 8.4.3.1, table 8-94
    MacExtendedAddress([u8; 8]),
    MacAssociationPermit(bool),
    MacPanId(u16),
    MacShortAddress(u16),
}

#[allow(dead_code)]
impl<Rng, U, TIMER, Config> MacService<'_, Rng, U, TIMER, Config>
where
    Rng: RngCore,
    U: UpperLayer,
    TIMER: DelayNs + Clone,
    Config: DriverConfig,
{
    /// Used by the next higher layer to attempt to write the given value to
    /// the indicated MAC PIB attribute.
    ///
    /// * `attribute` - Attribute to write
    pub(crate) async fn mlme_set_request(
        &mut self,
        attribute: &SetRequestAttribute,
    ) -> Result<(), SetError> {
        match attribute {
            SetRequestAttribute::MacPanId(pan_id) => self.pib.pan_id = *pan_id,
            SetRequestAttribute::MacShortAddress(short_address) => {
                self.pib.short_address = *short_address
            }
            SetRequestAttribute::MacExtendedAddress(extended_address) => {
                self.pib.extended_address = Some(*extended_address)
            }
            SetRequestAttribute::MacAssociationPermit(association_permit) => {
                self.pib.association_permit = *association_permit
            }
        }
        Ok(())
    }
}
