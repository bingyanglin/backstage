use anyhow::anyhow;
use async_trait::async_trait;
use backstage::{launcher, launcher::*, *};
use log::info;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

//////////////////////////////// HelloWorld Actor ////////////////////////////////////////////

// The HelloWorld actor's event type
#[derive(Serialize, Deserialize)]
pub enum HelloWorldEvent {
    Shutdown,
}

// The possible errors that a HelloWorld actor can have
#[derive(Error, Debug)]
pub enum HelloWorldError {
    #[error("Something went wrong")]
    SomeError,
}

// In order for an actor to make use of a custom error type,
// it should be convertable to an `ActorError` with an
// associated `ActorRequest` specifying how the supervisor
// should handle the error.
impl Into<ActorError> for HelloWorldError {
    fn into(self) -> ActorError {
        ActorError::RuntimeError(ActorRequest::Finish)
    }
}

// This is an example of a manual builder implementation.
// See the `build_howdy` fn below for a proc_macro implementation.
#[derive(Debug, Default, Clone)]
pub struct HelloWorldBuilder {
    name: String,
    num: u32,
}

impl HelloWorldBuilder {
    pub fn new(name: String, num: u32) -> Self {
        Self { name, num }
    }
}

impl ActorBuilder for HelloWorldBuilder {
    type BuiltActor = HelloWorld;

    fn build<E, S>(self, service: Service) -> HelloWorld
    where
        S: 'static + Send + EventHandle<E>,
    {
        let (name, num) = (self.name, self.num);
        let (sender, inbox) = tokio::sync::mpsc::unbounded_channel::<HelloWorldEvent>();
        HelloWorld {
            inbox,
            sender: HelloWorldSender(sender),
            service,
            name_num: format!("{}-{}", name, num),
        }
    }
}

// A wrapper type for a simple tokio channel which is used to pass
// events to the actor. This implements EventHandle so it can be used
// by other actors without knowing details about how this actor
// implements event handling.
#[derive(Debug, Clone)]
pub struct HelloWorldSender(UnboundedSender<HelloWorldEvent>);

impl EventHandle<HelloWorldEvent> for HelloWorldSender {
    fn send(&mut self, message: HelloWorldEvent) -> anyhow::Result<()> {
        self.0.send(message).map_err(|e| anyhow!(e.to_string()))
    }

    fn shutdown(&mut self) -> anyhow::Result<()> {
        self.send(HelloWorldEvent::Shutdown)
    }

    fn update_status(&mut self, _service: Service) -> anyhow::Result<()> {
        todo!()
    }
}

// The HelloWorld actor's state, which holds
// data created by a builder when the actor
// is spawned.
#[derive(Debug)]
pub struct HelloWorld {
    sender: HelloWorldSender,
    inbox: UnboundedReceiver<HelloWorldEvent>,
    service: Service,
    name_num: String,
}

// The Actor implementation, which defines how this actor will
// behave.
#[async_trait]
impl<E, S> Actor<E, S> for HelloWorld
where
    S: 'static + Send + EventHandle<E>,
{
    type Error = HelloWorldError;

    fn service(&mut self) -> &mut Service {
        &mut self.service
    }

    async fn init(&mut self, _supervisor: &mut S) -> Result<(), Self::Error> {
        info!("Initializing {}!", self.service.name);
        Ok(())
    }

    // This actor simply waits for a shutdown signal and then exits
    async fn run(&mut self, _supervisor: &mut S) -> Result<(), Self::Error> {
        info!("Running {}!", self.service.name);
        while let Some(evt) = self.inbox.recv().await {
            match evt {
                HelloWorldEvent::Shutdown => {
                    break;
                }
            }
        }
        Ok(())
    }

    async fn shutdown(&mut self, status: Result<(), Self::Error>, _supervisor: &mut S) -> Result<ActorRequest, ActorError> {
        info!("Shutting down {}!", self.service.name);
        match status {
            std::result::Result::Ok(_) => Ok(ActorRequest::Finish),
            std::result::Result::Err(e) => Err(e.into()),
        }
    }
}

impl<E, S> EventActor<E, S> for HelloWorld
where
    S: 'static + Send + EventHandle<E>,
{
    type Event = HelloWorldEvent;
    type Handle = HelloWorldSender;

    fn handle(&self) -> Self::Handle {
        self.sender.clone()
    }
}

//////////////////////////////// Howdy Actor ////////////////////////////////////////////

// Below is another actor type, which is identical is most ways to HelloWorld.
// However, it uses the proc_macro `build` to define the HowdyBuilder and it will
// intentionally timeout while shutting down.

#[derive(Serialize, Deserialize)]
pub enum HowdyEvent {
    Shutdown,
}
#[derive(Error, Debug)]
pub enum HowdyError {
    #[error("Something went wrong")]
    SomeError,
}

impl Into<ActorError> for HowdyError {
    fn into(self) -> ActorError {
        ActorError::RuntimeError(ActorRequest::Finish)
    }
}

#[build]
#[derive(Debug, Clone)]
pub fn build_howdy<LauncherEvent, LauncherSender>(service: Service) -> Howdy {
    let (sender, inbox) = tokio::sync::mpsc::unbounded_channel::<HowdyEvent>();
    Howdy {
        inbox,
        sender: HowdySender(sender),
        service,
    }
}

#[derive(Debug, Clone)]
pub struct HowdySender(UnboundedSender<HowdyEvent>);

impl EventHandle<HowdyEvent> for HowdySender {
    fn send(&mut self, message: HowdyEvent) -> anyhow::Result<()> {
        self.0.send(message).map_err(|e| anyhow!(e.to_string()))
    }

    fn shutdown(&mut self) -> anyhow::Result<()> {
        self.send(HowdyEvent::Shutdown)
    }

    fn update_status(&mut self, _service: Service) -> anyhow::Result<()> {
        todo!()
    }
}

#[derive(Debug)]
pub struct Howdy {
    sender: HowdySender,
    inbox: UnboundedReceiver<HowdyEvent>,
    service: Service,
}

#[async_trait]
impl<E, S> Actor<E, S> for Howdy
where
    S: 'static + Send + EventHandle<E>,
{
    type Error = HowdyError;

    fn service(&mut self) -> &mut Service {
        &mut self.service
    }

    async fn init(&mut self, _supervisor: &mut S) -> Result<(), Self::Error> {
        info!("Initializing {}!", self.service.name);
        Ok(())
    }

    async fn run(&mut self, _supervisor: &mut S) -> Result<(), Self::Error> {
        info!("Running {}!", self.service.name);
        while let Some(evt) = self.inbox.recv().await {
            match evt {
                HowdyEvent::Shutdown => {
                    break;
                }
            }
        }
        Ok(())
    }

    async fn shutdown(&mut self, status: Result<(), Self::Error>, _supervisor: &mut S) -> Result<ActorRequest, ActorError> {
        info!("Shutting down {}!", self.service.name);
        // Some process that takes longer than the defined timeout
        tokio::time::sleep(Duration::from_secs(4)).await;
        match status {
            std::result::Result::Ok(_) => Ok(ActorRequest::Finish),
            std::result::Result::Err(e) => Err(e.into()),
        }
    }
}

impl<E, S> EventActor<E, S> for Howdy
where
    S: 'static + Send + EventHandle<E>,
{
    type Event = HowdyEvent;
    type Handle = HowdySender;

    fn handle(&self) -> Self::Handle {
        self.sender.clone()
    }
}

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();

    startup().await.unwrap();
}

async fn startup() -> anyhow::Result<()> {
    launcher!(HelloWorldBuilder, HowdyBuilder)
        .add("HelloWorld", HelloWorldBuilder::new("HelloWorld".to_owned(), 1), &["Howdy"], None)?
        .name("Howdy")
        .timeout(Duration::from_secs(2))
        .builder(HowdyBuilder::new())?
        .execute(|_launcher| {
            info!("Executing with launcher");
        })
        .execute_async(|launcher| async {
            info!("Executing async with launcher");
            launcher
        })
        .await
        .launch()
        .await?;
    Ok(())
}
