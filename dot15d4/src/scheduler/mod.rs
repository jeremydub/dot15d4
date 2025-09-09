#![allow(dead_code)]

use core::cell::RefCell;
use core::future;

use dot15d4_driver::radio::DriverConfig;
use dot15d4_util::sync::{Channel, HasAddress, Receiver, Sender};
use rand_core::RngCore;

use self::transmission::TransmissionTask;
use crate::driver::DriverRequestSender;
use crate::mac::MacBufferAllocator;
use crate::pib::Pib;
use crate::MacContext;
use crate::{driver::DriverService, service_config, service_tasks};

use paste::paste;

use crate::service::{
    Service, ServiceConfig, ServiceTask, ServiceTaskEvent, ServiceTaskTransition,
};

mod csma;
mod transmission;
mod tsch;

/// The number of scheduler service requests that may be handed out in parallel.
const SCHEDULER_REQUESTS_CAPACITY: usize = 1;

/// The number of additional messages that may be pending.
const SCHEDULER_MSG_BACKLOG: usize = 1;

/// The number of upper layer tasks that receive indications.
///
/// Note: Currently we assume a single data channel terminated by a smoltcp
///       client. But this may change once we expose one or more independent
///       control channels towards applications directly.
const SCHEDULER_NUM_CLIENTS: usize = 1;

pub type SchedulerRequestChannel<'svc, RadioDriverImpl: DriverConfig> = Channel<
    (),
    SchedulerServiceRequest<'svc, RadioDriverImpl>,
    SchedulerServiceResponse<'svc, RadioDriverImpl>,
    SCHEDULER_REQUESTS_CAPACITY,
    SCHEDULER_MSG_BACKLOG,
    1,
>;
pub type SchedulerRequestReceiver<'svc, RadioDriverImpl: DriverConfig> = Receiver<
    'svc,
    (),
    SchedulerServiceRequest<'svc, RadioDriverImpl>,
    SchedulerServiceResponse<'svc, RadioDriverImpl>,
    SCHEDULER_REQUESTS_CAPACITY,
    SCHEDULER_MSG_BACKLOG,
    1,
>;
pub type SchedulerRequestSender<'svc, RadioDriverImpl: DriverConfig> = Sender<
    'svc,
    (),
    SchedulerServiceRequest<'svc, RadioDriverImpl>,
    SchedulerServiceResponse<'svc, RadioDriverImpl>,
    SCHEDULER_REQUESTS_CAPACITY,
    SCHEDULER_MSG_BACKLOG,
    1,
>;

pub enum SchedulerServiceState {
    UsingCsmaCa(CsmaCaSchedulerState),
    UsingTsch(TschSchedulerState),
}

pub enum CsmaCaSchedulerState {
    Transmitting(Transmission),
    Receiving(Reception),
}

pub enum TschSchedulerState {
    TransmittingInDedicatedTimeslot(Transmission),
    ReceivingInDedicatedTimeslot(Reception),
    TransmittingInSharedTimeslot(Transmission),
    ReceivinInSharedTimeslot(Reception),
}

pub struct SchedulerService<'svc, RadioDriverImpl: DriverConfig> {
    /// Timer instance to wait until driver requests become pending.
    timer: RadioDriverImpl::Timer,
    /// Message buffer allocator
    buffer_allocator: MacBufferAllocator,
    /// Upper layer channel from which MAC requests are received.
    request_receiver: SchedulerRequestReceiver<'svc, RadioDriverImpl>,
    /// Channel to communicate with one or several radio drivers.
    driver_request_sender: DriverRequestSender<'svc>,
    /// Context shared among tasks, containing PIB.
    context: RefCell<MacContext<'svc, RadioDriverImpl>>,
}

impl<'svc, RadioDriverImpl: DriverConfig> SchedulerService<'svc, RadioDriverImpl>
where
    Self: Service<'svc>,
{
    /// Creates a new [`MacService<U, Timer, R>`].
    pub fn new(
        timer: RadioDriverImpl::Timer,
        context: &'svc RefCell<MacContext<'svc, RadioDriverImpl>>,
        buffer_allocator: MacBufferAllocator,
        request_receiver: SchedulerRequestReceiver<'svc, RadioDriverImpl>,
        // indication_sender: MacIndicationSender<'svc>,
        driver_request_sender: DriverRequestSender<'svc>,
    ) -> Self {
        Self {
            timer,
            buffer_allocator,
            request_receiver,
            // indication_sender,
            driver_request_sender,
            context,
        }
    }
    pub async fn run(&'svc mut self) {
        future::pending().await
    }
}

impl<'svc, RadioDriverImpl: DriverConfig + 'svc> Service<'svc>
    for SchedulerService<'svc, RadioDriverImpl>
{
    type State = SchedulerServiceState;
    fn next_request() {
        todo!()
    }

    fn handle_task_result() {
        todo!()
    }
}

impl<RadioDriverImpl: DriverConfig> HasAddress<()>
    for SchedulerServiceRequest<'_, RadioDriverImpl>
{
    fn matches(&self, _: &()) -> bool {
        true
    }
}

service_config!(SchedulerService, DriverService);
service_tasks!(SchedulerService, Transmission);
