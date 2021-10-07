// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use super::{
    Route,
    Shutdown,
};
use std::{
    any::TypeId,
    collections::HashMap,
};
use tokio::sync::RwLock;

/// The actor level position in the supervision tree
pub type Depth = usize;
/// The actor's unique scope id
pub type ScopeId = usize;
/// Partitioned Scopes RwLock hashmap
pub type PartitionedScopes = RwLock<HashMap<ScopeId, Scope>>;

lazy_static::lazy_static! {
    /// PROMETHEUS REGISTRY, used by backstage to register metrics
    pub static ref PROMETHEUS_REGISTRY: prometheus::Registry = {
        prometheus::Registry::new_custom(Some("Backstage".into()), None).expect("PROMETHEUS_REGISTRY to be created")
    };
    /// Backstage partition count, default is the thread count
    pub static ref BACKSTAGE_PARTITIONS: usize = {
        if let Ok(p) = std::env::var("BACKSTAGE_PARTITIONS") {
            p.parse().expect("Invalid BACKSTAGE_PARTITIONS env")
        } else {
            num_cpus::get() * 10
        }
    };
    /// Array of Partitioned Scopes
    pub static ref SCOPES: Vec<PartitionedScopes> = {
        let mut scopes = Vec::with_capacity(*BACKSTAGE_PARTITIONS);
        for _ in 0..*BACKSTAGE_PARTITIONS {
            scopes.push(RwLock::default());
        }
        scopes
    };
    /// Visible global resources, todo add the dir name
    pub static ref VISIBLE_DATA: RwLock<HashMap<TypeId, Vec<(Depth,ScopeId)>>> = {
        let data_map = HashMap::new();
        RwLock::new(data_map)
    };
}

/// Resource blanket trait, forces Clone + Send + Sync + 'static bounds on any resource
pub trait Resource: Clone + Send + Sync + 'static {}
impl<T> Resource for T where T: Clone + Send + Sync + 'static {}

/// Data for given T: resource, along with subscribers
pub struct Data<T: Resource> {
    pub(crate) resource: Option<T>,
    pub(crate) subscribers: HashMap<ScopeId, Subscriber<T>>,
}

impl<T: Resource> Data<T> {
    /// Create new data from resource, used when the resource is created before it's being requested.
    /// Resource publisher will invoke this method
    pub fn with_resource(resource: T) -> Self {
        Self {
            resource: Some(resource),
            subscribers: HashMap::new(),
        }
    }
    /// Create data from subscriber, used when the resource is requested before it's being created.
    pub fn with_subscriber(scope_id: ScopeId, subscriber: Subscriber<T>) -> Self {
        let mut subscribers = HashMap::new();
        subscribers.insert(scope_id, subscriber);
        Self {
            resource: None,
            subscribers,
        }
    }
}
/// Subscriber variants
pub enum Subscriber<T: Resource> {
    /// OneCopy subscriber will receive one copy of the resource once it's available
    OneCopy(tokio::sync::oneshot::Sender<anyhow::Result<T>>),
    /// LinkedOneCopy subscriber will receive one copy of the resource once it's available,
    /// subscriber will get shutdown if the resource is replaced or dropped.
    LinkedOneCopy(
        Option<tokio::sync::oneshot::Sender<anyhow::Result<T>>>,
        Box<dyn Shutdown>,
    ),
    /// Subscriber will receive dynamic copies, pushed by the publisher,
    /// and None will be pushed if the resource got dropped by the publisher.
    DynCopy(Box<dyn Route<Option<T>>>),
}

impl<T: Resource> Subscriber<T> {
    /// Create subscriber for one copy
    pub fn one_copy(one_shot: tokio::sync::oneshot::Sender<anyhow::Result<T>>) -> Self {
        Self::OneCopy(one_shot)
    }
    /// Create linked subscriber for one copy
    pub fn linked_one_copy(
        one_shot: tokio::sync::oneshot::Sender<anyhow::Result<T>>,
        shutdown_handle: Box<dyn Shutdown>,
    ) -> Self {
        Self::LinkedOneCopy(Some(one_shot), shutdown_handle)
    }
    /// Create subscriber for dynamic copies.
    pub fn dyn_copy(boxed_route: Box<dyn Route<Option<T>>>) -> Self {
        Self::DynCopy(boxed_route)
    }
}
/// Cleanup object for T resource
pub(crate) struct Cleanup<T: Resource> {
    _marker: std::marker::PhantomData<T>,
    resource_scope_id: ScopeId,
}
impl<T: Resource> Cleanup<T> {
    /// Create Cleanup object, to cleanup the resource from any other scopes
    pub(crate) fn new(resource_scope_id: ScopeId) -> Self {
        Self {
            _marker: std::marker::PhantomData::<T>,
            resource_scope_id,
        }
    }
}
#[async_trait::async_trait]
/// Cleanup/unsubscribe from other
pub trait CleanupFromOther: Send + Sync {
    /// Cleanup subscriber from other scopes
    async fn cleanup_from_other(self: Box<Self>, my_scope_id: ScopeId);
}
#[async_trait::async_trait]
impl<T: Resource> CleanupFromOther for Cleanup<T> {
    async fn cleanup_from_other(self: Box<Self>, my_scope_id: ScopeId) {
        let resource_scopes_index = self.resource_scope_id % num_cpus::get();
        let mut lock = SCOPES[resource_scopes_index].write().await;
        if let Some(resource_scope) = lock.get_mut(&self.resource_scope_id) {
            if let Some(data) = resource_scope.data_and_subscribers.get_mut::<Data<T>>() {
                data.subscribers.remove(&my_scope_id);
            }
        };
        drop(lock);
    }
}

#[async_trait::async_trait]
/// Cleanup resource from self scope
pub trait CleanupSelf: Send + Sync {
    /// Cleanup the resource from the self scope
    async fn cleanup_self(self: Box<Self>, data_and_subscribers: &mut anymap::Map<dyn anymap::any::Any + Send + Sync>);
}
/// Cleanup self data struct
pub struct CleanupData<T: Resource> {
    _marker: std::marker::PhantomData<T>,
}
impl<T: Resource> CleanupData<T> {
    /// Create cleanup data struct
    pub(crate) fn new() -> Self {
        Self {
            _marker: std::marker::PhantomData::<T>,
        }
    }
}
#[async_trait::async_trait]
impl<T: Resource> CleanupSelf for CleanupData<T> {
    async fn cleanup_self(self: Box<Self>, data_and_subscribers: &mut anymap::Map<dyn anymap::any::Any + Send + Sync>) {
        if let Some(mut data) = data_and_subscribers.remove::<Data<T>>() {
            for (_sub_scope_id, subscriber) in data.subscribers.drain() {
                match subscriber {
                    Subscriber::OneCopy(one_sender) => {
                        one_sender.send(Err(anyhow::Error::msg("Cleanup"))).ok();
                    }
                    Subscriber::LinkedOneCopy(mut one_sender_opt, shutdown_handle) => {
                        if let Some(one_sender) = one_sender_opt.take() {
                            one_sender.send(Err(anyhow::Error::msg("Cleanup"))).ok();
                        }
                        shutdown_handle.shutdown().await;
                    }
                    Subscriber::DynCopy(boxed_route) => {
                        boxed_route.send_msg(None).await.ok();
                    }
                }
            }
        }
    }
}

/// The Actor's Scope, acts as global context for the actor.
/// - Enables any actor to shutdown any scope
/// - Provides the pub/sub functionality
/// - Form the supervision tree as file system directory
pub struct Scope {
    // the parent_id might be useful to traverse backward
    pub(crate) parent_id: Option<ScopeId>,
    pub(crate) shutdown_handle: Box<dyn Shutdown>,
    // to cleanup any self dyn subscribers from other scopes for a given type_id,
    // the scopeId here is the resource scope_id, this modified by the owner of the scope only.
    pub(crate) cleanup: HashMap<(TypeId, ScopeId), Box<dyn CleanupFromOther>>,
    // to cleanup self created data from the data_and_subscribers anymap
    pub(crate) cleanup_data: HashMap<TypeId, Box<dyn CleanupSelf>>,
    /// The create data along with their subscribers
    pub(crate) data_and_subscribers: anymap::Map<dyn anymap::any::Any + Send + Sync>,
    /// Active directories of the children
    pub(crate) active_directories: HashMap<String, ScopeId>,
    /// Supported dynamic routes by the actor (scope's owner)
    pub(crate) router: anymap::Map<dyn anymap::any::Any + Send + Sync>,
}

impl Scope {
    /// Create new scope
    pub fn new(parent_id: Option<ScopeId>, shutdown_handle: Box<dyn Shutdown>) -> Self {
        Self {
            parent_id,
            shutdown_handle,
            cleanup: HashMap::new(),
            cleanup_data: HashMap::new(),
            data_and_subscribers: anymap::Map::new(),
            active_directories: HashMap::new(),
            router: anymap::Map::new(),
        }
    }
    /// Lookup for a resource in a scope data store
    pub async fn lookup<T: Resource>(resource_scope_id: ScopeId) -> Option<T> {
        let resource_scopes_index = resource_scope_id % *BACKSTAGE_PARTITIONS;
        let mut lock = SCOPES[resource_scopes_index].write().await;
        if let Some(resource_scope) = lock.get_mut(&resource_scope_id) {
            if let Some(data) = resource_scope.data_and_subscribers.get::<Data<T>>() {
                data.resource.clone()
            } else {
                None
            }
        } else {
            None
        }
    }
}
