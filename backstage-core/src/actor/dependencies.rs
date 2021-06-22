use crate::{Act, Actor, Res, RuntimeScope, Sys, System};
use async_trait::async_trait;
use std::marker::PhantomData;
use tokio::sync::broadcast;

/// A dependency's status
pub enum DepStatus<T> {
    /// The dependency is ready to be used
    Ready(T),
    /// The dependency is not ready, here is a channel to await
    Waiting(broadcast::Receiver<PhantomData<T>>),
}

/// Defines dependencies that an actor or system can check for
#[async_trait]
pub trait Dependencies {
    /// Request a notification when a specific resource is ready
    async fn request(scope: &RuntimeScope) -> DepStatus<Self>
    where
        Self: 'static + Send + Sync + Sized,
    {
        match Self::instantiate(scope).await {
            Ok(dep) => DepStatus::Ready(dep),
            Err(_) => DepStatus::Waiting(
                if let Some(sender) = scope.get_data::<broadcast::Sender<PhantomData<Self>>>().await {
                    sender.subscribe()
                } else {
                    let (sender, receiver) = broadcast::channel::<PhantomData<Self>>(8);
                    scope.add_data(sender).await;
                    receiver
                },
            ),
        }
    }

    /// Instantiate instances of some dependencies
    async fn instantiate(scope: &RuntimeScope) -> anyhow::Result<Self>
    where
        Self: Sized;
}

#[async_trait]
impl<S: 'static + System + Send + Sync> Dependencies for Sys<S> {
    async fn instantiate(scope: &RuntimeScope) -> anyhow::Result<Self> {
        scope
            .system()
            .await
            .ok_or_else(|| anyhow::anyhow!("Missing system dependency: {}", std::any::type_name::<S>()))
    }
}

#[async_trait]
impl<A: Actor + Send + Sync> Dependencies for Act<A> {
    async fn instantiate(scope: &RuntimeScope) -> anyhow::Result<Self> {
        scope
            .actor_event_handle()
            .await
            .ok_or_else(|| anyhow::anyhow!("Missing actor dependency: {}", std::any::type_name::<A>()))
    }
}

#[async_trait]
impl<R: 'static + Send + Sync + Clone> Dependencies for Res<R> {
    async fn instantiate(scope: &RuntimeScope) -> anyhow::Result<Self> {
        scope
            .resource()
            .await
            .ok_or_else(|| anyhow::anyhow!("Missing resource dependency: {}", std::any::type_name::<R>()))
    }
}

#[async_trait]
impl Dependencies for () {
    async fn instantiate(_scope: &RuntimeScope) -> anyhow::Result<Self> {
        Ok(())
    }
}

macro_rules! impl_dependencies {
    ($($gen:ident),+) => {
        #[async_trait]
        impl<$($gen),+> Dependencies for ($($gen),+,)
        where $($gen: Dependencies + Send + Sync),+
        {
            async fn request(scope: &RuntimeScope) -> DepStatus<Self>
            where
                Self: 'static + Send + Sync,
            {
                let mut receivers = anymap::Map::<dyn anymap::any::Any + Send + Sync>::new();
                let mut total = 0;
                let mut ready = 0;
                $(
                    total += 1;
                    let status = $gen::request(scope).await;
                    match status {
                        DepStatus::Ready(_) => {
                            ready += 1;
                        }
                        DepStatus::Waiting(receiver) => {
                            receivers.insert(receiver);
                        }
                    }
                )+
                if total == ready {
                    DepStatus::Ready(($(receivers.remove::<$gen>().unwrap()),+,))
                } else {
                    let (sender, receiver) = broadcast::channel::<PhantomData<Self>>(8);
                    tokio::task::spawn(async move {
                        $(
                            let mut receiver = receivers.remove::<broadcast::Receiver<PhantomData<$gen>>>().unwrap();
                            if let Err(_) = receiver.recv().await {
                                return;
                            }

                        )+
                        sender.send(PhantomData).ok();
                    });
                    DepStatus::Waiting(receiver)
                }
            }

            async fn instantiate(scope: &RuntimeScope) -> anyhow::Result<Self>
            {
                Ok(($($gen::instantiate(scope).await?),+,))
            }
        }
    };
}

impl_dependencies!(A);
impl_dependencies!(A, B);
impl_dependencies!(A, B, C);
impl_dependencies!(A, B, C, D);
impl_dependencies!(A, B, C, D, E);
impl_dependencies!(A, B, C, D, E, F);
impl_dependencies!(A, B, C, D, E, F, G);
impl_dependencies!(A, B, C, D, E, F, G, H);
