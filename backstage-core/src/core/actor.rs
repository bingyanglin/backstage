// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use super::{
    rt::Rt,
    ActorResult,
    Channel,
    SupHandle,
};
use async_trait::async_trait;

/// The all-important Actor trait. This defines an Actor and what it do.
#[async_trait]
pub trait Actor<S: SupHandle<Self>>: Sized + Send + Sync + 'static {
    /// Allows specifying an actor's startup dependencies.
    type Data: Send + Sync + 'static;
    /// The type of channel this actor will use to receive events
    type Channel: Channel;
    /// Used to initialize the actor.
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> Result<Self::Data, super::Reason>;
    /// The main function for the actor
    async fn run(&mut self, rt: &mut Rt<Self, S>, data: Self::Data) -> ActorResult;
    /// Get this actor's type name
    fn type_name() -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Shutdown contract , should be implemented on the handle
#[async_trait::async_trait]
pub trait Shutdown: Send + 'static + Sync + dyn_clone::DynClone {
    async fn shutdown(&self);
    fn scope_id(&self) -> super::ScopeId;
}

dyn_clone::clone_trait_object!(Shutdown);

/// Defines the Shutdown event variant
pub trait ShutdownEvent: Send {
    /// Return Shutdown variant
    fn shutdown_event() -> Self;
}

/// Null supervisor, with no-ops
pub struct NullSupervisor;
#[async_trait::async_trait]
impl<T: Send + 'static> SupHandle<T> for NullSupervisor {
    type Event = ();
    // End of life for Actor of type T, invoked on shutdown.
    async fn eol(
        self,
        _scope_id: super::ScopeId,
        _service: super::Service,
        _actor: T,
        _r: super::ActorResult,
    ) -> Option<()> {
        Some(())
    }
}

#[async_trait::async_trait]
impl<T: Send> super::Report<T> for NullSupervisor {
    /// Report any status & service changes
    async fn report(&self, _scope_id: super::ScopeId, _service: super::Service) -> Option<()> {
        Some(())
    }
}

// test
#[cfg(test)]
mod tests {
    use crate::core::{
        Actor,
        ActorResult,
        IntervalChannel,
        Reason,
        Rt,
        StreamExt,
    };

    struct PrintHelloEveryFewMs;
    #[async_trait::async_trait]
    impl<S> Actor<S> for PrintHelloEveryFewMs
    where
        S: super::SupHandle<Self>,
    {
        type Data = ();
        type Channel = IntervalChannel<100>;
        async fn init(&mut self, _rt: &mut Rt<Self, S>) -> Result<Self::Data, Reason> {
            Ok(())
        }
        async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult {
            while let Some(_) = rt.inbox_mut().next().await {
                println!("HelloWorld")
            }
            Ok(())
        }
    }
}
