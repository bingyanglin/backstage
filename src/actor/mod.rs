use std::{
    borrow::Cow,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;

use futures::future::{AbortRegistration, Abortable, Aborted};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    time::error::Elapsed,
};

mod launcher;
mod registry;

pub use launcher::*;
pub(crate) use log::{error, info, warn};
pub use registry::*;

#[derive(Clone)]
pub enum Event<A: Clone + Send + Sync> {
    StartApp(A),
    ShutdownApp(A),
    // AknShutdown(AppsStates, Result<(), Need>),
    RequestService(A),
    StatusChange(Service),
    // Passthrough(AppsEvents, String),
    ExitProgram { using_ctrl_c: bool },
}

pub trait AddressBook {
    type Identifier;

    fn insert<M: 'static + Mailbox + Send>(&mut self, mailbox: M) -> Self::Identifier;

    fn get<M: 'static + Mailbox + Send>(&self, id: Self::Identifier) -> Option<&M>;

    fn remove<M: 'static + Mailbox + Send>(&mut self, id: Self::Identifier) -> Option<M>;
}

pub trait Mailbox {
    type Mail;
    type Mailbox;
    type SendError;

    fn get_mailbox(&self) -> &Self::Mailbox;

    fn send(&self, mail: Self::Mail) -> Result<(), Self::SendError>;
}

pub type NeedResult = Result<(), Need>;

#[derive(Copy, Clone, Debug)]
pub enum ResultSource {
    Init(NeedResult),
    Loop(NeedResult),
    Teardown(NeedResult),
}

pub trait Addressable<A: Clone + Send + Sync> {
    fn address(&self) -> A;
}

#[async_trait]
pub trait Sendable<A: Clone + Send + Sync> {
    type SendError;
    async fn send<E: 'static + Clone + Send + Sync>(&self, address: A, event: E) -> Result<(), Self::SendError>;
}

#[async_trait]
pub trait Init<A: Clone + Send + Sync> {
    async fn init(&mut self, supervisor: Option<A>) -> NeedResult;
}

#[async_trait]
pub trait EventLoop<A: Clone + Send + Sync> {
    async fn event_loop(&mut self, supervisor: Option<A>) -> NeedResult;
}

#[async_trait]
pub trait Teardown<A: Clone + Send + Sync> {
    async fn teardown(&mut self, status: ResultSource, supervisor: Option<A>) -> NeedResult;
}

#[async_trait]
pub trait Start<A: 'static + Clone + Send + Sync>: Init<A> + EventLoop<A> + Teardown<A> {
    async fn start(&mut self, supervisor: Option<A>) -> ResultSource {
        let mut res = self.init(supervisor.clone()).await;
        let mut source = ResultSource::Init(res);
        if res.is_ok() {
            res = self.event_loop(supervisor.clone()).await;
            source = ResultSource::Loop(res);
            if res.is_ok() {
                res = self.teardown(ResultSource::Loop(res), supervisor).await;
                source = ResultSource::Teardown(res);
            } else {
                self.teardown(source, supervisor).await.ok();
            }
        } else {
            self.teardown(source, supervisor).await.ok();
        }
        source
    }
}

#[async_trait]
pub trait StartAbortable<A: 'static + Clone + Send + Sync>: Init<A> + EventLoop<A> + Teardown<A> {
    /// This method will start the actor with abortable event loop.
    /// ```
    /// let (abort_handle, abort_registration) = AbortHandle::new_pair();
    /// ```
    async fn start_abortable(&mut self, abort_registration: AbortRegistration, supervisor: Option<A>) -> ResultSource {
        let mut res = self.init(supervisor.clone()).await;
        let mut source = ResultSource::Init(res);
        if res.is_ok() {
            let abortable_event_loop_fut = Abortable::new(self.event_loop(supervisor.clone()), abort_registration);
            match abortable_event_loop_fut.await {
                Ok(mut res) => {
                    if res.is_ok() {
                        res = self.teardown(ResultSource::Loop(res), supervisor).await;
                        source = ResultSource::Teardown(res);
                    } else {
                        self.teardown(source, supervisor).await.ok();
                    }
                }
                Err(aborted) => {
                    res = Err(Need::Abort);
                    source = ResultSource::Loop(res);
                    self.aborted(aborted, supervisor.clone());
                    self.teardown(source, supervisor).await.ok();
                }
            }
        } else {
            self.teardown(source, supervisor).await.ok();
        }
        source
    }

    fn aborted(&self, aborted: Aborted, supervisor: Option<A>);
}

#[async_trait]
pub trait StartTimeout<A: 'static + Clone + Send + Sync>: Init<A> + EventLoop<A> + Teardown<A> {
    /// This method will start the actor with timeout/ttl event loop by using
    /// the runtime timer timeout functionality.
    async fn start_timeout(&mut self, duration: Duration, supervisor: Option<A>) -> ResultSource {
        let mut res = self.init(supervisor.clone()).await;
        let mut source = ResultSource::Init(res);
        if res.is_ok() {
            let timeout_event_loop_fut = tokio::time::timeout(duration, self.event_loop(supervisor.clone()));
            match timeout_event_loop_fut.await {
                Ok(mut res) => {
                    if res.is_ok() {
                        res = self.teardown(ResultSource::Loop(res), supervisor).await;
                        source = ResultSource::Teardown(res);
                    } else {
                        self.teardown(source, supervisor).await.ok();
                    }
                }
                Err(elapsed) => {
                    res = Err(Need::Abort);
                    source = ResultSource::Loop(res);
                    self.timed_out(elapsed, supervisor.clone());
                    self.teardown(source, supervisor).await.ok();
                }
            }
        } else {
            self.teardown(source, supervisor).await.ok();
        }
        source
    }

    fn timed_out(&self, elapsed: Elapsed, supervisor: Option<A>);
}

#[async_trait]
pub trait Actor<A: 'static + Clone + Send + Sync>: Send {
    fn name(&self) -> Cow<str> {
        Cow::from(stringify!(Self))
    }

    async fn init(&mut self, supervisor: Option<A>) -> NeedResult;

    async fn event_loop(&mut self, supervisor: Option<A>) -> NeedResult;

    async fn teardown(&mut self, status: ResultSource, supervisor: Option<A>) -> NeedResult;

    fn aborted(&self, aborted: Aborted, supervisor: Option<A>);

    fn timed_out(&self, elapsed: Elapsed, supervisor: Option<A>);

    async fn start(mut self: Box<Self>, supervisor: Option<A>) -> ResultSource {
        let mut res = self.init(supervisor.clone()).await;
        let mut source = ResultSource::Init(res);
        if res.is_ok() {
            res = self.event_loop(supervisor.clone()).await;
            source = ResultSource::Loop(res);
            if res.is_ok() {
                res = self.teardown(ResultSource::Loop(res), supervisor).await;
                source = ResultSource::Teardown(res);
            } else {
                self.teardown(source, supervisor).await.ok();
            }
        } else {
            self.teardown(source, supervisor).await.ok();
        }
        source
    }

    /// This method will start the actor with abortable event loop.
    /// ```
    /// let (abort_handle, abort_registration) = AbortHandle::new_pair();
    /// ```
    async fn start_abortable(mut self: Box<Self>, abort_registration: AbortRegistration, supervisor: Option<A>) -> ResultSource {
        let mut res = self.init(supervisor.clone()).await;
        let mut source = ResultSource::Init(res);
        if res.is_ok() {
            let abortable_event_loop_fut = Abortable::new(self.event_loop(supervisor.clone()), abort_registration);
            match abortable_event_loop_fut.await {
                Ok(mut res) => {
                    if res.is_ok() {
                        res = self.teardown(ResultSource::Loop(res), supervisor).await;
                        source = ResultSource::Teardown(res);
                    } else {
                        self.teardown(source, supervisor).await.ok();
                    }
                }
                Err(aborted) => {
                    res = Err(Need::Abort);
                    source = ResultSource::Loop(res);
                    self.aborted(aborted, supervisor.clone());
                    self.teardown(source, supervisor).await.ok();
                }
            }
        } else {
            self.teardown(source, supervisor).await.ok();
        }
        source
    }

    /// This method will start the actor with timeout/ttl event loop by using
    /// the runtime timer timeout functionality.
    async fn start_timeout(mut self: Box<Self>, duration: Duration, supervisor: Option<A>) -> ResultSource {
        let mut res = self.init(supervisor.clone()).await;
        let mut source = ResultSource::Init(res);
        if res.is_ok() {
            let timeout_event_loop_fut = tokio::time::timeout(duration, self.event_loop(supervisor.clone()));
            match timeout_event_loop_fut.await {
                Ok(mut res) => {
                    if res.is_ok() {
                        res = self.teardown(ResultSource::Loop(res), supervisor).await;
                        source = ResultSource::Teardown(res);
                    } else {
                        self.teardown(source, supervisor).await.ok();
                    }
                }
                Err(elapsed) => {
                    res = Err(Need::Abort);
                    source = ResultSource::Loop(res);
                    self.timed_out(elapsed, supervisor.clone());
                    self.teardown(source, supervisor).await.ok();
                }
            }
        } else {
            self.teardown(source, supervisor).await.ok();
        }
        source
    }
}

macro_rules! actor {
    ($t:ty) => {
        #[async_trait]
        impl<A> Actor<A> for $t
        where
            $t: Start<A> + StartTimeout<A> + StartAbortable<A> + Send,
            A: 'static + Clone + Send + Sync,
        {
            async fn init(&mut self, supervisor: Option<A>) -> NeedResult {
                self.init(supervisor).await
            }

            async fn event_loop(&mut self, supervisor: Option<A>) -> NeedResult {
                self.event_loop(supervisor).await
            }

            async fn teardown(&mut self, status: ResultSource, supervisor: Option<A>) -> NeedResult {
                self.teardown(status, supervisor).await
            }

            fn aborted(&self, aborted: Aborted, supervisor: Option<A>) {
                self.aborted(aborted, supervisor)
            }

            fn timed_out(&self, elapsed: Elapsed, supervisor: Option<A>) {
                self.timed_out(elapsed, supervisor)
            }
        }
    };
}

#[repr(u8)]
#[derive(Clone, PartialEq, Debug)]
pub enum ServiceStatus {
    /// Early bootup
    Starting = 0,
    /// Late bootup
    Initializing = 1,
    /// The service is operational but one or more services failed(Degraded or Maintenance) or in process of being
    /// fully operational while startup.
    Degraded = 2,
    /// The service is fully operational.
    Running = 3,
    /// The service is shutting down, should be handled accordingly by active dependent services
    Stopping = 4,
    /// The service is maintenance mode, should be handled accordingly by active dependent services
    Maintenance = 5,
    /// The service is not running, should be handled accordingly by active dependent services
    Stopped = 6,
}

#[derive(Clone, Debug)]
pub struct Service {
    /// The status of the service
    pub status: ServiceStatus,
    /// Service name (ie app name or microservice name)
    pub name: String,
    /// Timestamp since the service is up
    pub up_since: SystemTime,
    /// Total milliseconds when the app service has been offline since up_since
    pub downtime_ms: u64,
    /// inner services similar
    pub microservices: std::collections::HashMap<String, Self>,
    /// Optional log file path
    pub log_path: Option<String>,
}

impl Service {
    pub fn new() -> Self {
        Self {
            status: ServiceStatus::Starting,
            name: String::new(),
            up_since: SystemTime::now(),
            downtime_ms: 0,
            microservices: std::collections::HashMap::new(),
            log_path: None,
        }
    }
    pub fn set_status(mut self, service_status: ServiceStatus) -> Self {
        self.status = service_status;
        self
    }
    pub fn update_status(&mut self, service_status: ServiceStatus) {
        self.status = service_status;
    }
    pub fn set_name(mut self, name: String) -> Self {
        self.name = name;
        self
    }
    pub fn update_name(&mut self, name: String) {
        self.name = name;
    }
    pub fn get_name(&self) -> String {
        self.name.clone()
    }
    pub fn set_downtime_ms(mut self, downtime_ms: u64) -> Self {
        self.downtime_ms = downtime_ms;
        self
    }
    pub fn set_log(mut self, log_path: String) -> Self {
        self.log_path = Some(log_path);
        self
    }
    pub fn update_microservice(&mut self, service_name: String, microservice: Self) {
        self.microservices.insert(service_name, microservice);
    }
    pub fn update_microservice_status(&mut self, service_name: &str, status: ServiceStatus) {
        self.microservices.get_mut(service_name).unwrap().status = status;
    }
    pub fn delete_microservice(&mut self, service_name: &str) {
        self.microservices.remove(service_name);
    }
    pub fn is_stopping(&self) -> bool {
        self.status == ServiceStatus::Stopping
    }
    pub fn is_stopped(&self) -> bool {
        self.status == ServiceStatus::Stopped
    }
    pub fn is_running(&self) -> bool {
        self.status == ServiceStatus::Running
    }
    pub fn is_initializing(&self) -> bool {
        self.status == ServiceStatus::Initializing
    }
    pub fn is_maintenance(&self) -> bool {
        self.status == ServiceStatus::Maintenance
    }
    pub fn is_degraded(&self) -> bool {
        self.status == ServiceStatus::Degraded
    }
    pub fn service_status(&self) -> &ServiceStatus {
        &self.status
    }
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Need {
    Restart,
    RescheduleAfter(Duration),
    Abort,
}
