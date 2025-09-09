pub mod mcps;
pub mod neighbors;

use dot15d4_driver::radio::DriverConfig;
use dot15d4_util::sync::HasAddress;
use paste::paste;
use rand_core::RngCore;

use crate::driver::DriverRequestSender;
use crate::scheduler::SchedulerRequestSender;
use crate::service::{
    MpscServiceRunner, Service, ServiceConfig, ServiceTask, ServiceTaskEvent, ServiceTaskTransition,
};
use crate::{
    driver::{constants::PHY_MAX_PACKET_SIZE_127, radio::MAX_DRIVER_OVERHEAD},
    pib::Pib,
    scheduler::SchedulerService,
};
use crate::{service_config, service_tasks, MacContext};
use core::cell::RefCell;
use core::future;

pub use dot15d4_frame as frame;
use dot15d4_util::{
    allocator::BufferAllocator,
    sync::{Channel, Receiver, Sender},
};

use mcps::data::DataRequestTask;

// TODO: Make allocator and channel capacities and the number of upper layer
//       tasks configurable.

/// The max number of UL Rx tokens that may be handed out in parallel.
const UL_MAX_RX_TOKENS: usize = 1;

/// The max number of UL Tx tokens that may be handed out in parallel.
/// Note: Each Rx token requires an accompanying Tx token to be allocated.
const UL_MAX_TX_TOKENS: usize = 1 + UL_MAX_RX_TOKENS;

/// The number of additional messages that may be pending.
/// Note: 1 is currently the min number supported.
const UL_MSG_BACKLOG: usize = 1;

/// The number of upper layer tasks that receive indications.
///
/// Note: Currently we assume a single data channel terminated by a smoltcp
///       client. But this may change once we expose one or more independent
///       control channels towards applications directly.
const UL_NUM_CLIENTS: usize = 1;

pub type MacRequestChannel<'svc, RadioDriverImpl: DriverConfig> = Channel<
    (),
    MacServiceRequest<'svc, RadioDriverImpl>,
    MacServiceResponse<'svc, RadioDriverImpl>,
    UL_MAX_TX_TOKENS,
    UL_MSG_BACKLOG,
    1,
>;
pub type MacRequestReceiver<'svc, RadioDriverImpl: DriverConfig> = Receiver<
    'svc,
    (),
    MacServiceRequest<'svc, RadioDriverImpl>,
    MacServiceResponse<'svc, RadioDriverImpl>,
    UL_MAX_TX_TOKENS,
    UL_MSG_BACKLOG,
    1,
>;
pub type MacRequestSender<'svc, RadioDriverImpl: DriverConfig> = Sender<
    'svc,
    (),
    MacServiceRequest<'svc, RadioDriverImpl>,
    MacServiceResponse<'svc, RadioDriverImpl>,
    UL_MAX_TX_TOKENS,
    UL_MSG_BACKLOG,
    1,
>;

/// TODO: handle data indication
const MAC_NUM_INDICATION_TASKS: usize = 1;

// TODO: Challenge the following capacity calculation.
/// Buffers are allocated by:
/// - tx token
/// - indication task
/// - driver service (2 pre-allocated buffers for RX/TX ACKs)
///
/// Required buffers:
/// - one buffer per max outstanding upper layer tx token (= max request tasks)
/// - one buffer per indication task
/// - one pre-allocated buffer for outgoing ACKs
/// - one pre-allocated buffer for incoming ACKs
pub const MAC_NUM_REQUIRED_BUFFERS: usize = UL_MAX_TX_TOKENS + MAC_NUM_INDICATION_TASKS + 2;
pub const MAC_BUFFER_SIZE: usize = PHY_MAX_PACKET_SIZE_127 + MAX_DRIVER_OVERHEAD;

pub type MacBufferAllocator = BufferAllocator;

pub enum MacServiceState {
    WaitingForRequest,
}

pub struct MacService<'svc, RadioDriverImpl: DriverConfig> {
    // runner: MpscServiceRunner<'svc, Self>,
    /// Message buffer allocator
    buffer_allocator: MacBufferAllocator,
    /// Upper layer channel from which MAC requests are received.
    request_receiver: MacRequestReceiver<'svc, RadioDriverImpl>,
    /// Channel to communicate with scheduler service.
    scheduler_request_sender: DriverRequestSender<'svc>,
    /// Context shared among tasks, containing PIB.
    context: RefCell<MacContext<'svc, RadioDriverImpl>>,
}

impl<'svc, RadioDriverImpl: DriverConfig + 'svc> Service<'svc>
    for MacService<'svc, RadioDriverImpl>
{
    type State = MacServiceState;
    fn next_request() {
        todo!()
    }

    fn handle_task_result() {
        todo!()
    }
}

impl<'svc, RadioDriverImpl: DriverConfig> MacService<'svc, RadioDriverImpl>
where
    Self: Service<'svc>,
{
    /// Creates a new [`MacService<U, Timer, R>`].
    pub fn new(
        context: &'svc RefCell<MacContext<'svc, RadioDriverImpl>>,
        buffer_allocator: MacBufferAllocator,
        request_receiver: MacRequestReceiver<'svc, RadioDriverImpl>,
        // indication_sender: MacIndicationSender<'svc>,
        scheduler_request_sender: SchedulerRequestSender<'svc, RadioDriverImpl>,
    ) -> Self {
        Self {
            context,
            buffer_allocator,
            request_receiver,
            scheduler_request_sender,
        }
    }
    pub async fn run(&'svc mut self) {
        future::pending().await
    }
}

impl<RadioDriverImpl: DriverConfig> HasAddress<()> for MacServiceRequest<'_, RadioDriverImpl> {
    fn matches(&self, _: &()) -> bool {
        true
    }
}

service_config!(MacService, SchedulerService);
service_tasks!(MacService, DataRequest);
