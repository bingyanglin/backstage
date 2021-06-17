use crate::{actor::shutdown_stream::ShutdownStream, Actor, Channel, Dependencies, Sender, SupervisorEvent, System};
use anymap::{any::CloneAny, Map};
use async_trait::async_trait;
use futures::{
    future::{AbortHandle, Abortable, BoxFuture},
    StreamExt,
};
use lru::LruCache;
#[cfg(feature = "rand_pool")]
use rand::Rng;
use std::{
    cell::RefCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::Arc,
};
use tokio::{
    sync::{oneshot, RwLock},
    task::JoinHandle,
};

pub use basic_runtime::*;
pub use full_runtime::*;
pub use scopes::*;
pub use system_runtime::*;

mod basic_runtime;
mod full_runtime;
mod scopes;
mod system_runtime;

/// Defines the bare essentials of a scoped runtime which can spawn and
/// manage actors as well as shut them down appropriately.
#[async_trait]
pub trait BaseRuntime: Send + Sync {
    /// Get the join handles of this runtime's scoped tasks
    fn join_handles(&self) -> &Vec<JoinHandle<anyhow::Result<()>>>;

    /// Mutably get the join handles of this runtime's scoped tasks
    fn join_handles_mut(&mut self) -> &mut Vec<JoinHandle<anyhow::Result<()>>>;

    /// Get the shutdown handles of this runtime's scoped tasks
    fn shutdown_handles(&self) -> &Vec<(Option<oneshot::Sender<()>>, AbortHandle)>;

    /// Mutably get the shutdown handles of this runtime's scoped tasks
    fn shutdown_handles_mut(&mut self) -> &mut Vec<(Option<oneshot::Sender<()>>, AbortHandle)>;

    /// Get the anymap of senders (handles) for this runtime's scoped tasks
    fn senders(&self) -> &Map<dyn CloneAny + Send + Sync>;

    /// Mutably get the anymap of senders (handles) for this runtime's scoped tasks
    fn senders_mut(&mut self) -> &mut Map<dyn CloneAny + Send + Sync>;

    /// Create a child of this runtime
    fn child(&self) -> Self;

    /// Create a new scope within this one
    async fn scope<O: Send + Sync, F: Send + FnOnce(&mut RuntimeScope<'_, Self>) -> O>(&mut self, f: F) -> anyhow::Result<O>
    where
        Self: 'static + Sized,
    {
        let res = f(&mut RuntimeScope::new(self));
        self.join().await;
        Ok(res)
    }

    /// Shutdown the tasks in this runtime's scope
    fn shutdown(&mut self) {
        for handle in self.shutdown_handles_mut().iter_mut() {
            handle.0.take().map(|h| h.send(()));
        }
    }

    /// Await the tasks in this runtime's scope
    async fn join(&mut self) {
        for handle in self.join_handles_mut().drain(..) {
            handle.await.ok();
        }
    }

    /// Abort the tasks in this runtime's scope. This will shutdown tasks that have
    /// shutdown handles instead.
    fn abort(mut self)
    where
        Self: Sized,
    {
        for handles in self.shutdown_handles_mut().iter_mut() {
            if let Some(shutdown_handle) = handles.0.take() {
                if let Err(_) = shutdown_handle.send(()) {
                    handles.1.abort();
                }
            } else {
                handles.1.abort();
            }
        }
    }

    /// Get an event handle of the given type, if it exists in this scope
    fn event_handle<T: 'static + Clone + Send + Sync>(&self) -> Option<T> {
        self.senders().get::<T>().map(|handle| handle.clone())
    }

    /// Get an actor's event handle, if it exists in this scope.
    /// Note: This will only return a handle if the actor exists outside of a pool.
    fn actor_event_handle<A: Actor>(&self) -> Option<Act<A>>
    where
        Self: Sized,
    {
        self.senders()
            .get::<<A::Channel as Channel<A::Event>>::Sender>()
            .map(|handle| Act(handle.clone()))
    }

    /// Send an event to a given actor, if it exists in this scope
    async fn send_actor_event<A: Actor>(&mut self, event: A::Event) -> anyhow::Result<()>
    where
        Self: Sized,
    {
        let handle = self
            .senders_mut()
            .get_mut::<<A::Channel as Channel<A::Event>>::Sender>()
            .ok_or_else(|| anyhow::anyhow!("No channel for this actor!"))?;
        Sender::<A::Event>::send(handle, event).await
    }
}

/// A runtime which manages systems in addition to actors
#[async_trait]
pub trait SystemRuntime: BaseRuntime {
    /// Get the anymap of systems for this runtime's scope
    fn systems(&self) -> &Map<dyn CloneAny + Send + Sync>;

    /// Mutably get the anymap of systems for this runtime's scope
    fn systems_mut(&mut self) -> &mut Map<dyn CloneAny + Send + Sync>;

    /// Get a shared reference to a system if it exists in this runtime's scope
    fn system<S: 'static + System + Send + Sync>(&self) -> Option<Sys<S>>
    where
        Self: Sized,
    {
        self.systems().get::<Arc<RwLock<S>>>().map(|sys| Sys(sys.clone()))
    }

    /// Get a system's event handle if the system exists in this runtime's scope
    fn system_event_handle<S: System>(&self) -> Option<<S::Channel as Channel<S::ChildEvents>>::Sender>
    where
        Self: Sized,
    {
        self.senders()
            .get::<<S::Channel as Channel<S::ChildEvents>>::Sender>()
            .map(|handle| handle.clone())
    }

    /// Send an event to a system if it exists within this runtime's scope
    async fn send_system_event<S: System>(&mut self, event: S::ChildEvents) -> anyhow::Result<()>
    where
        Self: Sized,
    {
        let handle = self
            .senders_mut()
            .get_mut::<<S::Channel as Channel<S::ChildEvents>>::Sender>()
            .ok_or_else(|| anyhow::anyhow!("No channel for this actor!"))?;
        Sender::<S::ChildEvents>::send(handle, event).await
    }
}

/// A runtime which manages shared resources
pub trait ResourceRuntime: BaseRuntime {
    /// Get the runtime scope's shared resources anymap
    fn resources(&self) -> &Map<dyn CloneAny + Send + Sync>;

    /// Mutably get the runtime scope's shared resources anymap
    fn resources_mut(&mut self) -> &mut Map<dyn CloneAny + Send + Sync>;

    /// Get a shared resource if it exists in this runtime's scope
    fn resource<R: 'static + Send + Sync + Clone>(&self) -> Option<Res<R>> {
        self.resources().get::<R>().map(|res| Res(res.clone()))
    }
}

/// A runtime which manages pools of actors
pub trait PoolRuntime: BaseRuntime {
    /// Get the anymap of pools from this runtimes's scope
    fn pools(&self) -> &Map<dyn CloneAny + Send + Sync>;

    /// Mutably get the anymap of pools from this runtimes's scope
    fn pools_mut(&mut self) -> &mut Map<dyn CloneAny + Send + Sync>;

    /// Get the pool of a specified actor if it exists in this runtime's scope
    fn pool<A>(&self) -> Option<Res<Arc<RwLock<ActorPool<A>>>>>
    where
        Self: 'static + Sized,
        A: 'static + Actor + Send + Sync,
    {
        self.pools().get::<Arc<RwLock<ActorPool<A>>>>().map(|pool| Res(pool.clone()))
    }
}

/// A shared resource
pub struct Res<R: Clone>(R);

impl<R: Deref + Clone> Deref for Res<R> {
    type Target = R::Target;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<R: DerefMut + Clone> DerefMut for Res<R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

/// A shared system reference
pub struct Sys<S: System>(Arc<RwLock<S>>);

impl<S: System> Deref for Sys<S> {
    type Target = RwLock<S>;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

/// An actor handle, used to send events
pub struct Act<A: Actor>(<A::Channel as Channel<A::Event>>::Sender);

impl<A: Actor> Deref for Act<A> {
    type Target = <A::Channel as Channel<A::Event>>::Sender;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<A: Actor> DerefMut for Act<A> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A pool of actors which can be queried for actor handles
pub struct ActorPool<A: Actor> {
    handles: Vec<Rc<RefCell<<A::Channel as Channel<A::Event>>::Sender>>>,
    lru: LruCache<usize, Rc<RefCell<<A::Channel as Channel<A::Event>>::Sender>>>,
}

impl<A: Actor> Clone for ActorPool<A> {
    fn clone(&self) -> Self {
        let handles = self.handles.iter().map(|rc| Rc::new(RefCell::new(rc.borrow().clone()))).collect();
        let mut lru = LruCache::unbounded();
        for (idx, lru_rc) in self.lru.iter().rev() {
            lru.put(*idx, Rc::new(RefCell::new(lru_rc.borrow().clone())));
        }
        Self { handles: handles, lru }
    }
}

unsafe impl<A: Actor + Send> Send for ActorPool<A> {}

unsafe impl<A: Actor + Sync> Sync for ActorPool<A> {}

impl<A: Actor> Default for ActorPool<A> {
    fn default() -> Self {
        Self {
            handles: Default::default(),
            lru: LruCache::unbounded(),
        }
    }
}

impl<A: Actor> ActorPool<A> {
    fn push(&mut self, handle: <A::Channel as Channel<A::Event>>::Sender) {
        let idx = self.handles.len();
        let handle_rc = Rc::new(RefCell::new(handle));
        self.handles.push(handle_rc.clone());
        self.lru.put(idx, handle_rc);
    }

    /// Get the least recently used actor handle from this pool
    pub fn get_lru(&mut self) -> Option<Act<A>> {
        self.lru.pop_lru().map(|(idx, handle)| {
            let res = handle.borrow().clone();
            self.lru.put(idx, handle);
            Act(res)
        })
    }

    /// Send to the least recently used actor handle in this pool
    pub async fn send_lru(&mut self, event: A::Event) -> anyhow::Result<()> {
        if let Some(mut handle) = self.get_lru() {
            handle.send(event).await
        } else {
            anyhow::bail!("No handles in pool!");
        }
    }

    #[cfg(feature = "rand_pool")]
    /// Get a random actor handle from this pool
    pub fn get_random(&mut self) -> Option<Act<A>> {
        let mut rng = rand::thread_rng();
        self.handles
            .get(rng.gen_range(0..self.handles.len()))
            .map(|rc| Act(rc.borrow().clone()))
    }

    #[cfg(feature = "rand_pool")]
    /// Send to a random actor handle from this pool
    pub async fn send_random(&mut self, event: A::Event) -> anyhow::Result<()> {
        if let Some(mut handle) = self.get_random() {
            handle.send(event).await
        } else {
            anyhow::bail!("No handles in pool!");
        }
    }

    /// Get an iterator over the actor handles in this pool
    pub fn iter(&mut self) -> std::vec::IntoIter<Act<A>> {
        self.handles
            .iter()
            .map(|rc| Act(rc.borrow().clone()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Send to every actor handle in this pool
    pub async fn send_all(&mut self, event: A::Event) -> anyhow::Result<()>
    where
        A::Event: Clone,
    {
        for mut handle in self.iter() {
            handle.send(event.clone()).await?;
        }
        Ok(())
    }
}
