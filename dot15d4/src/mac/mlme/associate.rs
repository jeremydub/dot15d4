use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

use crate::mac::MacService;
use crate::upper::UpperLayer;

pub struct AssociateConfirm;

#[allow(dead_code)]
impl<Rng, U, TIMER> MacService<'_, Rng, U, TIMER>
where
    Rng: RngCore,
    U: UpperLayer,
    TIMER: DelayNs + Clone,
{
    /// Requests the association with a coordinator.
    async fn mlme_associate_request(&self) -> Result<AssociateConfirm, ()> {
        // TODO: support association
        Err(())
    }
}
