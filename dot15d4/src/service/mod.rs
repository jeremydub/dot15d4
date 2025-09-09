mod runner;

pub use runner::MpscServiceRunner;

pub trait Service<'svc>: ServiceConfig<'svc>
where
    Self: Sized,
{
    type State;

    fn next_request();
    fn handle_task_result();
}

pub trait ServiceConfig<'svc>
where
    Self: Sized,
{
    type Request;
    type Response;
    type LowerService: ServiceConfig<'svc>;
}

pub trait ServiceTask<'svc, Svc: Service<'svc>> {
    ///
    type Request;
    /// A task MAY produce intermediate and final results while being executed.
    type Result;
    type Error;

    /// This method must be called by the task executor (i.e. the service)
    /// whenever the task becomes pending. It drives the task state machine
    /// until it terminates (see [`TaskTransition::Terminated`]).
    ///
    /// A task becomes pending when one of the following events occurs:
    /// - [`TaskEvent::Entry`]: The task has just been initialized.
    /// - [`TaskEvent::SchedSvcResponse`]: The service returned a
    ///   response to a pending request from the state machine.
    fn step(self, event: ServiceTaskEvent<'svc, Svc>) -> ServiceTaskTransition<'svc, Svc, Self>
    where
        Self: Sized;
}

/// Represents the transition triggered by a task step.
///
/// A transition produces an lower service request and additionally intermediate
/// and/or final task results.
pub enum ServiceTaskTransition<'svc, Svc: Service<'svc>, Task: ServiceTask<'svc, Svc>> {
    /// Signals to the executor that a transition has been
    /// triggered, i.e. an lower service request needs to be sent to the
    /// service.
    ///
    /// A transition MAY yield an intermediate result to be handled by the
    /// executor.
    ///
    /// A transition MAY time out.
    Intermediate(
        /// The task's next state.
        Task,
        /// The lower service request produced by the transition.
        <Svc::LowerService as ServiceConfig<'svc>>::Request,
        /// An optional intermediate task result.
        Option<Task::Result>,
    ),

    /// Signals to the executor that the state machine exited, possibly with a
    /// final result.
    Terminated(Result<Task::Result, Task::Error>),
}

/// The set of events that may occur while executing a task state machine.
pub enum ServiceTaskEvent<'svc, Svc: Service<'svc>> {
    /// Event produced once by the executor immediately after a (sub-)state
    /// machine has been instantiated. Takes the state machine from the initial
    /// pseudostate to its initial state.
    Entry,

    /// The service has produced a response to the lower service request
    /// previously produced by the state machine's request.
    LowerServiceResponse(<Svc::LowerService as ServiceConfig<'svc>>::Response),
}

#[macro_export]
macro_rules! service_config_no_lower {
    ($service:ident) => {
        paste! {
            impl<'svc, RadioDriverImpl: DriverConfig + 'svc> ServiceConfig<'svc> for $service<'svc, RadioDriverImpl>
            {
                type Request = [<$service Request>]<'svc, RadioDriverImpl>;
                type Response = [<$service Response>]<'svc, RadioDriverImpl>;
                type LowerService = ();
            }
        }
    };
}

#[macro_export]
macro_rules! service_config {
    ($service:ident, $lower_service:ident) => {
        paste! {
            impl<'svc, RadioDriverImpl: DriverConfig + 'svc> ServiceConfig<'svc> for $service<'svc, RadioDriverImpl>
            {
                type Request = [<$service Request>]<'svc, RadioDriverImpl>;
                type Response = [<$service Response>]<'svc, RadioDriverImpl>;
                type LowerService = $lower_service<'svc, RadioDriverImpl>;
            }
        }
    };
}

#[macro_export]
macro_rules! service_tasks {
    ($service:ident, $($task:ident),+)=> {
        paste!{

            pub(crate) enum [<$service Task>]<'svc, RadioDriverImpl: DriverConfig> {
                $($task([<$task Task>]<'svc, RadioDriverImpl>)),*
            }

            pub enum [<$service Request>]<'svc, RadioDriverImpl: DriverConfig + 'svc> {
                $($task(<[<$task Task>]<'svc, RadioDriverImpl> as ServiceTask<'svc, $service<'svc, RadioDriverImpl>>>::Request)),*
            }

            pub enum [<$service Response>]<'svc, RadioDriverImpl: DriverConfig + 'svc> {
                $($task(Result<<[<$task Task>]<'svc, RadioDriverImpl> as ServiceTask<'svc, $service<'svc, RadioDriverImpl>>>::Result, <[<$task Task>]<'svc, RadioDriverImpl> as ServiceTask<'svc, $service<'svc, RadioDriverImpl>>>::Error>)),*
            }

            enum [<$service TaskResult>]<'svc, RadioDriverImpl: DriverConfig + 'svc> {
                $($task(<[<$task Task>]<'svc, RadioDriverImpl> as ServiceTask<'svc, $service<'svc, RadioDriverImpl>>>::Result)),*
            }

            enum [<$service TaskError>]<'svc, RadioDriverImpl: DriverConfig + 'svc> {
                $($task(<[<$task Task>]<'svc, RadioDriverImpl> as ServiceTask<'svc, $service<'svc, RadioDriverImpl>>>::Error)),*
            }

            $(service_tasks!(transition_converter: $service, $task);)*

            impl<'svc, RadioDriverImpl: DriverConfig + 'svc> ServiceTask<'svc, $service<'svc, RadioDriverImpl>> for [<$service Task>]<'svc, RadioDriverImpl> {
                type Request = [<$service Request>]<'svc, RadioDriverImpl>;
                type Result = [<$service TaskResult>]<'svc, RadioDriverImpl>;
                type Error = [<$service TaskError>]<'svc, RadioDriverImpl>;

                fn step(self, event: ServiceTaskEvent<'svc, $service<'svc, RadioDriverImpl>>) -> ServiceTaskTransition<'svc, $service<'svc, RadioDriverImpl>, Self> {
                    match self {
                        $([<$service Task>]::$task(inner_task) => inner_task.step(event).into()),*
                    }
                }
            }
        }
    };

    (transition_converter: $service:ident, $task:ident) => {
        paste!{
            impl<'svc, RadioDriverImpl: DriverConfig + 'svc> From<ServiceTaskTransition<'svc, $service<'svc, RadioDriverImpl>,[<$task Task>]<'svc, RadioDriverImpl>>> for ServiceTaskTransition<'svc, $service<'svc,RadioDriverImpl>, [<$service Task>]<'svc, RadioDriverImpl>> {
                fn from(value: ServiceTaskTransition<'svc, $service<'svc, RadioDriverImpl>,[<$task Task>]<'svc, RadioDriverImpl>>) -> Self {
                    match value {
                        ServiceTaskTransition::Intermediate(updated_task, driver_request, task_result) => {
                            let updated_task = [<$service Task>]::$task(updated_task);
                            let task_result = task_result.map(|task_result| [<$service TaskResult>]::$task(task_result)) ;
                            ServiceTaskTransition::Intermediate(updated_task, driver_request, task_result)
                        },
                        ServiceTaskTransition::Terminated(task_result) => {
                            ServiceTaskTransition::Terminated(match task_result {
                                Ok(task_result) => Ok([<$service TaskResult>]::$task(task_result)),
                                Err(task_error) => Err([<$service TaskError>]::$task(task_error))
                            })
                        },
                    }
                }
            }
        }
    }
}

impl<'svc> Service<'svc> for () {
    type State = ();
    fn next_request() {}
    fn handle_task_result() {}
}

impl<'svc> ServiceConfig<'svc> for () {
    type Request = ();
    type Response = ();
    type LowerService = ();
}

impl<'svc> ServiceTask<'svc, ()> for () {
    type Request = ();
    type Result = ();
    type Error = ();

    fn step(self, _: ServiceTaskEvent<'svc, ()>) -> ServiceTaskTransition<'svc, (), Self>
    where
        Self: Sized,
    {
        ServiceTaskTransition::Terminated(Ok(()))
    }
}
