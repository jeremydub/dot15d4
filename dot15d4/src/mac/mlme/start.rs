use crate::upper::UpperLayer;
use dot15d4_frame3::driver::DriverConfig;
use embedded_hal_async::delay::DelayNs;
use rand_core::RngCore;

use super::MacService;

struct StartConfirm {}

#[allow(dead_code)]
impl<Rng, U, TIMER, Config> MacService<'_, Rng, U, TIMER, Config>
where
    Rng: RngCore,
    U: UpperLayer,
    TIMER: DelayNs + Clone,
    Config: DriverConfig,
{
    /// Used by PAN coordinator to initiate a new PAN or to begin using a new
    /// configuration. Also used by a device already associated with an
    /// existing PAN to begin using a new configuration.
    async fn mlme_start_request(&mut self) -> Result<StartConfirm, ()> {
        Err(())
    }
}
