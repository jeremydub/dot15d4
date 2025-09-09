use core::cell::Cell;

use bitmaps::{Bits, BitsImpl};

use dot15d4_util::sync::{
    select, ConsumerToken, HasAddress, MatchingResponse, PollingResponseToken, Receiver,
    ResponseToken, Sender,
};

use crate::service::{Service, ServiceTask};

use super::{ServiceConfig, ServiceTaskEvent, ServiceTaskTransition};

pub trait ServiceRunner<'svc, Svc: Service<'svc>> {
    type State: Default;

    async fn run(&mut self) -> !;
}

pub trait ServiceScheduler<'svc, Svc: Service<'svc>> {
    /// Next request (or task) to poll, among outstanding service requests
    async fn next_service_request(&mut self) -> Svc::Request;
    async fn handle_service_response(&self, response: Svc::Response);
}

pub enum MpscRunnerState<'svc, Svc: Service<'svc>> {
    Initial,
    WaitingForMessage(ConsumerToken),
    ReceivingServiceRequest(ConsumerToken, Svc::Request, ResponseToken),
    ReceivingLowerServiceResponse(
        ConsumerToken,
        <Svc::LowerService as ServiceConfig<'svc>>::Response,
        u8,
    ),
}

impl<'svc, Svc: Service<'svc>> Default for MpscRunnerState<'svc, Svc> {
    fn default() -> Self {
        MpscRunnerState::Initial
    }
}

pub struct MpscServiceRunner<
    'svc,
    Svc,
    Task,
    const CAPACITY: usize,
    const LOWER_CAPACITY: usize,
    const BACKLOG: usize = 1,
    const PARALLEL_SERVICE_REQUESTS: usize = 1,
> where
    Svc: Service<'svc>,
    Svc::Request: HasAddress<()>,
    <Svc::LowerService as ServiceConfig<'svc>>::Request: HasAddress<()>,
    Task: ServiceTask<'svc, Svc>,
    BitsImpl<CAPACITY>: Bits,
    BitsImpl<LOWER_CAPACITY>: Bits,
{
    /// Current state of the service runner
    state: Cell<Option<MpscRunnerState<'svc, Svc>>>,

    // Service request tasks are indexed by the message slots of the corresponding
    service_tasks: [Option<Task>; CAPACITY],

    // Outstanding lower-service requests will be pushed to this vector and polled for
    // responses.
    outstanding_lower_service_requests: heapless::Vec<PollingResponseToken, LOWER_CAPACITY>,

    // Mapping bewteen lower-service request and service Task. The index
    // corresponds to the lower-service message slot, the content to the
    // corresponding service request slot.
    lower_msg_slot_to_task_index: [usize; LOWER_CAPACITY],

    // Response tokens for outstanding service requests.
    outstanding_service_requests: [Option<ResponseToken>; PARALLEL_SERVICE_REQUESTS],

    // Receiver part of the channel communicating with upper service
    request_receiver: Receiver<'svc, (), Svc::Request, Svc::Response, CAPACITY, BACKLOG, 1>,

    // Sender part of the channel communicating with lower service
    lower_request_sender: Sender<
        'svc,
        (),
        <Svc::LowerService as ServiceConfig<'svc>>::Request,
        <Svc::LowerService as ServiceConfig<'svc>>::Response,
        LOWER_CAPACITY,
        BACKLOG,
        1,
    >,
}

impl<
        'svc,
        Svc,
        Task,
        const CAPACITY: usize,
        const LOWER_CAPACITY: usize,
        const BACKLOG: usize,
        const PARALLEL_SERVICE_REQUESTS: usize,
    >
    MpscServiceRunner<'svc, Svc, Task, CAPACITY, LOWER_CAPACITY, BACKLOG, PARALLEL_SERVICE_REQUESTS>
where
    Svc: Service<'svc>,
    Svc::Request: HasAddress<()>,
    <Svc::LowerService as ServiceConfig<'svc>>::Request: HasAddress<()>,
    Task: ServiceTask<'svc, Svc>,
    BitsImpl<CAPACITY>: Bits,
    BitsImpl<LOWER_CAPACITY>: Bits,
{
    pub fn new(
        request_receiver: Receiver<'svc, (), Svc::Request, Svc::Response, CAPACITY, BACKLOG, 1>,
        lower_request_sender: Sender<
            'svc,
            (),
            <Svc::LowerService as ServiceConfig<'svc>>::Request,
            <Svc::LowerService as ServiceConfig<'svc>>::Response,
            LOWER_CAPACITY,
            BACKLOG,
            1,
        >,
    ) -> Self {
        MpscServiceRunner {
            state: Cell::new(Some(MpscRunnerState::Initial)),
            service_tasks: [const { None }; CAPACITY],
            outstanding_lower_service_requests: heapless::Vec::new(),
            lower_msg_slot_to_task_index: [0; LOWER_CAPACITY],
            outstanding_service_requests: [const { None }; PARALLEL_SERVICE_REQUESTS],

            request_receiver,
            lower_request_sender,
        }
    }
}

impl<
        'svc,
        Svc,
        Task,
        const CAPACITY: usize,
        const LOWER_CAPACITY: usize,
        const BACKLOG: usize,
        const PARALLEL_SERVICE_REQUESTS: usize,
    > ServiceRunner<'svc, Svc>
    for MpscServiceRunner<
        'svc,
        Svc,
        Task,
        CAPACITY,
        LOWER_CAPACITY,
        BACKLOG,
        PARALLEL_SERVICE_REQUESTS,
    >
where
    Svc: Service<'svc>,
    Svc::Request: HasAddress<()>,
    <Svc::LowerService as ServiceConfig<'svc>>::Request: HasAddress<()>,
    Task: ServiceTask<'svc, Svc>,
    BitsImpl<CAPACITY>: Bits,
    BitsImpl<LOWER_CAPACITY>: Bits,
{
    type State = MpscRunnerState<'svc, Svc>;
    async fn run(&mut self) -> ! {
        let mut state = self.state.take().expect("already running");
        loop {
            state = self.next_state(state).await;
        }
    }
}

impl<
        'svc,
        Svc,
        Task,
        const CAPACITY: usize,
        const LOWER_CAPACITY: usize,
        const BACKLOG: usize,
        const PARALLEL_SERVICE_REQUESTS: usize,
    >
    MpscServiceRunner<'svc, Svc, Task, CAPACITY, LOWER_CAPACITY, BACKLOG, PARALLEL_SERVICE_REQUESTS>
where
    Svc: Service<'svc>,
    Svc::Request: HasAddress<()>,
    <Svc::LowerService as ServiceConfig<'svc>>::Request: HasAddress<()>,
    Task: ServiceTask<'svc, Svc>,
    BitsImpl<CAPACITY>: Bits,
    BitsImpl<LOWER_CAPACITY>: Bits,
{
    async fn next_state(
        &mut self,
        state: MpscRunnerState<'svc, Svc>,
    ) -> MpscRunnerState<'svc, Svc> {
        match state {
            MpscRunnerState::Initial => {
                let consumer_token = self
                    .request_receiver
                    .try_allocate_consumer_token()
                    .expect("capacity");
                MpscRunnerState::WaitingForMessage(consumer_token)
            }
            MpscRunnerState::WaitingForMessage(mut consumer_token) => {
                match select(
                    self.request_receiver
                        .wait_for_request(&mut consumer_token, &()),
                    self.lower_request_sender
                        .wait_for_response(&mut self.outstanding_lower_service_requests),
                )
                .await
                {
                    select::Either::First((response_token, service_request)) => {
                        MpscRunnerState::ReceivingServiceRequest(
                            consumer_token,
                            service_request,
                            response_token,
                        )
                    }
                    select::Either::Second(MatchingResponse {
                        response: lower_service_response,
                        msg_slot: lower_service_msg_slot,
                    }) => MpscRunnerState::ReceivingLowerServiceResponse(
                        consumer_token,
                        lower_service_response,
                        lower_service_msg_slot,
                    ),
                }
            }
            MpscRunnerState::ReceivingServiceRequest(
                consumer_token,
                service_request,
                response_token,
            ) => {
                //
                MpscRunnerState::WaitingForMessage(consumer_token)
            }
            MpscRunnerState::ReceivingLowerServiceResponse(
                consumer_token,
                lower_service_response,
                lower_service_msg_slot,
            ) => {
                //
                MpscRunnerState::WaitingForMessage(consumer_token)
            }
        }
    }

    fn step_task<'tasks>(
        &self,
        service_task_index: usize,
        service_task: Task,
        event: ServiceTaskEvent<'svc, Svc>,
    ) {
        let task_result = match service_task.step(event) {
            ServiceTaskTransition::Intermediate(
                updated_task,
                lower_request,
                intermediate_result,
            ) => {
                // Safety: We reserved sufficient channel capacity.
                let driver_msg_token = self
                    .lower_request_sender
                    .try_allocate_request_token()
                    .unwrap();
                let driver_response_token = self
                    .lower_request_sender
                    .send_request_polling_response(driver_msg_token, lower_request);
                self.lower_msg_slot_to_task_index[driver_response_token.message_slot() as usize] =
                    service_task_index;
                self.outstanding_lower_service_requests
                    .push(driver_response_token)
                    .unwrap();
                self.service_tasks[service_task_index] = Some(updated_task);
                intermediate_result
            }
            _ => unreachable!(), // ServiceTaskTransition::Terminated(task_result) => Some(task_result),
        };

        if let Some(task_result) = task_result {
            self.handle_request_task_result(
                task_result,
                self.outstanding_service_requests[service_task_index]
                    .take()
                    .unwrap(),
            );
        }
    }

    fn create_request_task(&'svc self, service_request: Svc::Request) -> Svc::Task {
        // match service_request {
        //     MacRequest::McpsDataRequest(data_request) => {
        //         MacSvcTask::DataRequest(DataRequestTask::new(data_request, &self.context))
        //     }
        //     MacRequest::MlmeBeaconRequest(_) => todo!(),
        //     MacRequest::MlmeSetRequest(_) => todo!(),
        // }
    }

    fn handle_request_task_result(
        &self,
        result: MacSvcTaskResult<RadioDriverImpl>,
        response_token: ResponseToken,
    ) {
        match result {
            MacSvcTaskResult::DataRequest(task_result) => {
                let recovered_radio_frame = match task_result {
                    DataRequestResult::Sent(recovered_radio_frame) => recovered_radio_frame,
                    DataRequestResult::CcaBusy(unsent_radio_frame)
                    | DataRequestResult::Nack(unsent_radio_frame) => {
                        // TODO: CSMA/CA or Retry.
                        unsent_radio_frame.forget_size::<RadioDriverImpl>()
                    }
                };

                // Safety: Clients must allocate buffers from the MAC's
                //         allocator.
                unsafe {
                    self.buffer_allocator
                        .deallocate_buffer(recovered_radio_frame.into_buffer());
                }

                // Safety: We signal reception _after_ de-allocating the buffer
                //         so that clients can use the reception signal to
                //         safely manage bounded buffer resources. We may even
                //         return the buffer at some time so that it doesn't
                //         have to be re-allocated. We just don't do that
                //         currently as the smoltcp driver is synchronous and
                //         cannot handle any response.
                self.request_receiver.received(response_token, ());
            }
            // The rest are indications
            _ => unreachable!(),
        }
    }
}
