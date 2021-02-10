use async_trait::async_trait;
use chronicle::*;
use log::{error, info, warn};
use std::{borrow::Cow, sync::Arc, time::Duration};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    Mutex,
};

#[derive(Clone)]
pub struct Sender<T: Clone + Send + Sync> {
    mailbox: tokio::sync::mpsc::UnboundedSender<T>,
}

impl<T: Clone + Send + Sync> Sender<T> {
    pub fn new(mailbox: tokio::sync::mpsc::UnboundedSender<T>) -> Self {
        Self { mailbox }
    }
}

impl<T: Clone + Send + Sync> Mailbox for Sender<T> {
    type Mail = T;

    type Mailbox = tokio::sync::mpsc::UnboundedSender<T>;

    type SendError = tokio::sync::mpsc::error::SendError<Self::Mail>;

    fn get_mailbox(&self) -> &Self::Mailbox {
        &self.mailbox
    }

    fn send(&self, mail: Self::Mail) -> Result<(), Self::SendError> {
        self.mailbox.send(mail)
    }
}

pub struct App {
    service: Service,
    address: Address,
    apps: Vec<Box<dyn chronicle::actor::App<Address>>>,
    app_count: usize,
    sender: UnboundedSender<Event<Address>>,
    receiver: UnboundedReceiver<Event<Address>>,
    consumed_ctrl_c: bool,
    registry: Arc<Mutex<Registry<Address>>>,
}

impl Addressable<Address> for App {
    fn address(&self) -> Address {
        self.address
    }
}

#[async_trait]
impl Launcher<Address> for App {
    fn get_main_sender(&self) -> &UnboundedSender<Event<Address>> {
        &self.sender
    }

    fn get_main_receiver(&mut self) -> &mut UnboundedReceiver<Event<Address>> {
        &mut self.receiver
    }

    fn get_service(&mut self) -> &mut Service {
        &mut self.service
    }

    fn get_address(&self) -> &Address {
        &self.address
    }

    fn start_app(&mut self, app: Address) {
        todo!()
    }

    fn shutdown_app(&mut self, app: Address) {
        todo!()
    }

    fn exit_program(&mut self, ctrl_c: bool) {
        todo!()
    }

    fn add_app<T: chronicle::actor::App<Address>>(mut self, app: T) -> Self {
        self.app_count += 1;
        self.apps.push(Box::new(app));
        self
    }

    fn get_all_apps(&mut self) -> &mut Vec<Box<dyn chronicle::App<Address>>> {
        &mut self.apps
    }
}

impl App {
    async fn build(registry: Arc<Mutex<Registry<Address>>>) -> Self {
        let (mailbox, inbox) = tokio::sync::mpsc::unbounded_channel::<Event<Address>>();
        let address = registry.lock().await.insert(Sender::new(mailbox.clone()));

        App {
            service: Service::default(),
            address,
            apps: Vec::new(),
            app_count: 0,
            sender: mailbox,
            receiver: inbox,
            consumed_ctrl_c: false,
            registry,
        }
    }
}

struct Listener {
    address: Address,
    speaker: Address,
    inbox: tokio::sync::mpsc::UnboundedReceiver<ListenerEvent>,
    service: Service,
    registry: Arc<Mutex<Registry<Address>>>,
}

impl Listener {
    async fn build(speaker: Address, registry: Arc<Mutex<Registry<Address>>>) -> Self {
        let (mailbox, inbox) = tokio::sync::mpsc::unbounded_channel::<ListenerEvent>();
        let address = registry.lock().await.insert(Sender::new(mailbox));

        Listener {
            address,
            speaker,
            inbox,
            service: Service::default(),
            registry,
        }
    }
}

struct Speaker {
    address: Address,
    inbox: tokio::sync::mpsc::UnboundedReceiver<SpeakerEvent>,
    service: Service,
    registry: Arc<Mutex<Registry<Address>>>,
}

impl Speaker {
    async fn build(registry: Arc<Mutex<Registry<Address>>>) -> Self {
        let (mailbox, inbox) = tokio::sync::mpsc::unbounded_channel::<SpeakerEvent>();
        let address = registry.lock().await.insert(Sender::new(mailbox));

        Speaker {
            address,
            inbox,
            service: Service::default(),
            registry,
        }
    }
}

#[derive(Clone)]
pub enum ListenerEvent {
    Heard(String),
    Shutdown,
}

#[derive(Clone)]
pub enum SpeakerEvent {
    Say(String),
    Shutdown,
}

#[async_trait]
impl Addressable<Address> for Listener {
    fn address(&self) -> Address {
        self.address
    }
}

#[async_trait]
impl Sendable<Address> for Listener {
    type SendError = Cow<'static, str>;

    async fn send<E: 'static + Clone + Send + Sync>(&self, address: Address, event: E) -> Result<(), Self::SendError> {
        if let Some(mailbox) = self.registry.lock().await.get::<Sender<E>>(address) {
            mailbox.send(event).map_err(|e| e.to_string().into())
        } else {
            Err(format!("No mailbox for Address {}", address).into())
        }
    }
}

#[async_trait]
impl Actor<Address> for Listener {
    async fn init(&mut self, _supervisor: Option<Address>) -> NeedResult {
        info!("Starting Listener");
        Ok(())
    }

    async fn event_loop(&mut self, _supervisor: Option<Address>) -> NeedResult {
        info!("Running Listener");
        let sayings = vec!["Hello", "Bonjour", "Salut"];
        let mut i = 0;
        loop {
            if let Err(e) = self.send(self.speaker, SpeakerEvent::Say(sayings[i].to_string())).await {
                error!("Error: {}", e);
            };
            i += 1;
            i %= sayings.len();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn teardown(&mut self, status: ResultSource, _supervisor: Option<Address>) -> NeedResult {
        info!("Shutting down Listener");
        self.send(self.speaker, SpeakerEvent::Shutdown).await.ok();
        Ok(())
    }

    fn aborted(&self, _aborted: futures::future::Aborted, _supervisor: Option<Address>) {
        warn!("Aborting Listener!")
    }

    fn timed_out(&self, _elapsed: tokio::time::error::Elapsed, _supervisor: Option<Address>) {}
}

#[async_trait]
impl Addressable<Address> for Speaker {
    fn address(&self) -> Address {
        self.address
    }
}

#[async_trait]
impl Sendable<Address> for Speaker {
    type SendError = Cow<'static, str>;

    async fn send<E: 'static + Clone + Send + Sync>(&self, address: Address, event: E) -> Result<(), Self::SendError> {
        if let Some(mailbox) = self.registry.lock().await.get::<Sender<E>>(address) {
            mailbox.send(event).map_err(|e| e.to_string().into())
        } else {
            Err(format!("No mailbox for Address {}", address).into())
        }
    }
}

#[async_trait]
impl Actor<Address> for Speaker {
    async fn init(&mut self, _supervisor: Option<Address>) -> NeedResult {
        info!("Starting Speaker");
        Ok(())
    }

    async fn event_loop(&mut self, _supervisor: Option<Address>) -> NeedResult {
        info!("Running Speaker");
        loop {
            if let Some(evt) = self.inbox.recv().await {
                match evt {
                    SpeakerEvent::Say(s) => {
                        info!("Listener heard \"{}\"", s);
                    }
                    SpeakerEvent::Shutdown => return Ok(()),
                }
            }
        }
    }

    async fn teardown(&mut self, status: ResultSource, _supervisor: Option<Address>) -> NeedResult {
        info!("Shutting down Speaker");
        Ok(())
    }

    fn aborted(&self, _aborted: futures::future::Aborted, _supervisor: Option<Address>) {
        warn!("Aborting Speaker!")
    }

    fn timed_out(&self, _elapsed: tokio::time::error::Elapsed, _supervisor: Option<Address>) {}
}

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();

    let registry = Arc::new(Mutex::new(Registry::default()));

    let speaker = Speaker::build(registry.clone()).await;
    let listener = Listener::build(speaker.address, registry.clone()).await;
    let app = Box::new(App::build(registry).await.add_app(speaker).add_app(listener));

    app.start(None).await;
}
