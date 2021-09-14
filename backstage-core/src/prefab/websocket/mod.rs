// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

/// Websocket listener
mod websocket_listener;
/// WebSocket receiver (incomings from client)
mod websocket_receiver;
/// WebSocket Sender (outcomings to client)
mod websocket_sender;

pub(crate) use websocket_listener::WebsocketListener;
pub(crate) use websocket_receiver::WebsocketReceiver;
pub(crate) use websocket_sender::{
    WebsocketSender,
    WebsocketSenderEvent,
};

#[derive(serde::Deserialize, serde::Serialize, Clone)]
/// The route message, wrapper around Message::Text inner string
pub struct RouteMessage(pub String);
impl From<RouteMessage> for String {
    fn from(v: RouteMessage) -> Self {
        v.0
    }
}
// Deserializable event
#[derive(serde::Deserialize, serde::Serialize)]
/// The ws interface event type
pub enum Event {
    /// shutdown the actor
    Shutdown,
    /// Cast event T using the pid's router, without responder.
    Cast(RouteMessage),
    /// Send event T using the pid's router.
    Call(RouteMessage),
    /// Request the service tree
    RequestServiceTree,
}

impl Event {
    /// Create shutdown event
    pub fn shutdown() -> Self {
        Self::Shutdown
    }
    /// Create cast event
    pub fn cast(message: String) -> Self {
        Self::Cast(RouteMessage(message))
    }
    /// Create request service tree event
    pub fn service() -> Self {
        Self::RequestServiceTree
    }
}
// Serializable response
#[derive(serde::Serialize)]
/// The expected client's response for a given event request
pub enum Response {
    /// Success shutdown signal
    Shutdown(ActorPath),
    /// Successful cast
    Sent(RouteMessage),
    /// Successful Response from a call
    Response(RouteMessage),
    /// Requested service tree
    ServiceTree(Service),
}

#[derive(serde::Serialize, serde::Deserialize)]
/// The expected client's error for a given event request
pub enum Error {
    /// Failed to shutdown the actor under the ActorPath
    /// (path, error string)
    Shutdown(ActorPath, String),
    /// Failed to cast RouteMessge to ActorPath.
    Cast(ActorPath, RouteMessage, String),
    /// Failed to call RouteMessge to ActorPath.
    Call(ActorPath, RouteMessage, String),
    /// Unable to fetch the service
    ServiceTree(String),
}

/// Wrapper around the final expected result
pub type ResponseResult = Result<Response, Error>;

#[derive(serde::Deserialize, serde::Serialize, Default, Clone)] // todo impl better ser/de
/// The actor directory path
pub struct ActorPath {
    root: ScopeId,
    path: Vec<String>,
}
impl ActorPath {
    /// Create new actor path, with root = 0 as the default scope_id
    pub fn new() -> Self {
        Self {
            root: 0,
            path: Vec::new(),
        }
    }
    /// Create actor path starting from the provided scope_id
    pub fn with_scope_id(scope_id: ScopeId) -> Self {
        Self {
            root: scope_id,
            path: Vec::new(),
        }
    }
    /// Push directory name to the actor path
    pub fn push(mut self, dir_name: String) -> Self {
        self.path.push(dir_name);
        self
    }
    /// Process the actor path and get the destination scope_id
    pub async fn destination(&self) -> Option<ScopeId> {
        let mut current_scope_id = self.root;
        let mut iter = self.path.iter();
        // traverse the scopes in seq order to reach the destination
        while let Some(dir_name) = iter.next() {
            let scopes_index = current_scope_id % *BACKSTAGE_PARTITIONS;
            let lock = SCOPES[scopes_index].read().await;
            if let Some(scope) = lock.get(&current_scope_id) {
                if let Some(new_current_scope_id) = scope.active_directories.get(dir_name) {
                    current_scope_id = *new_current_scope_id;
                    drop(lock)
                } else {
                    return None;
                }
            } else {
                return None;
            };
        }
        Some(current_scope_id)
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
/// The interface request, which is sent by the websocket clients
pub struct Interface {
    /// The targeted actor path
    pub actor_path: ActorPath,
    /// The event which we should send to the targeted actor
    pub event: Event,
}

impl Interface {
    /// Create new interface request with provided actor path and event
    pub fn new(actor_path: ActorPath, event: Event) -> Self {
        Self { actor_path, event }
    }
    /// Convert the interface request into tungstenite's message type
    pub fn to_message(self) -> tokio_tungstenite::tungstenite::Message {
        let json = serde_json::to_string(&self).expect("Serializeable json");
        Message::Text(json)
    }
}

use crate::core::*;
// Websocket supervisor

/// The websocket supervisor, enables websocket server and manages ws connections
pub struct Websocket {
    addr: std::net::SocketAddr,
    ttl: Option<u32>,
    link_to: Option<Box<dyn Shutdown>>,
}

impl Websocket {
    /// Create new Websocket struct
    pub fn new(addr: std::net::SocketAddr) -> Self {
        Self {
            addr,
            ttl: None,
            link_to: None,
        }
    }
    /// Set Time to live
    pub fn set_ttl(mut self, ttl: u32) -> Self {
        self.ttl.replace(ttl);
        self
    }
    /// Link handle to the websocket
    pub fn link_to(mut self, handle: Box<dyn Shutdown>) -> Self {
        self.link_to.replace(handle);
        self
    }
}

/// Websocket event type
pub enum WebsocketEvent {
    /// Shutdown variant, to receive shutdown signal
    Shutdown,
    /// Microservice variant to receive report and eol events
    Microservice(ScopeId, Service, Option<ActorResult<()>>),
    /// New TcpStream from listener
    TcpStream(tokio::net::TcpStream),
}

impl<T> ReportEvent<T> for WebsocketEvent {
    fn report_event(scope_id: ScopeId, service: Service) -> Self {
        Self::Microservice(scope_id, service, None)
    }
}

impl<T> EolEvent<T> for WebsocketEvent {
    fn eol_event(scope_id: ScopeId, service: Service, _actor: T, r: ActorResult<()>) -> Self {
        Self::Microservice(scope_id, service, Some(r))
    }
}

impl ShutdownEvent for WebsocketEvent {
    fn shutdown_event() -> Self {
        Self::Shutdown
    }
}

#[async_trait::async_trait]
impl<S: SupHandle<Self>> Actor<S> for Websocket {
    type Data = ();
    type Channel = UnboundedChannel<WebsocketEvent>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        let listener = WebsocketListener::new(self.addr, self.ttl);
        rt.start(None, listener).await?;
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()>
    where
        S: SupHandle<Self>,
    {
        while let Some(event) = rt.inbox_mut().next().await {
            match event {
                WebsocketEvent::Shutdown => {
                    rt.stop().await;
                    if rt.microservices_stopped() {
                        break;
                    }
                }
                WebsocketEvent::TcpStream(tcp_stream) => {
                    if let Ok(peer) = tcp_stream.peer_addr() {
                        if let Ok(ws_stream) = tokio_tungstenite::accept_async(tcp_stream).await {
                            let peer: String = peer.to_string();
                            let sender_name = format!("Sender@{}", peer);
                            let receiver_name = format!("Receiver@{}", peer);
                            let (split_sink, split_stream) = ws_stream.split();
                            let sender = WebsocketSender::new(split_sink);
                            if let Ok((sender_handle, _)) = rt.spawn(sender_name, sender).await {
                                let receiver = WebsocketReceiver::new(split_stream, sender_handle);
                                rt.spawn(receiver_name, receiver).await.ok();
                            }
                        }
                    }
                }
                WebsocketEvent::Microservice(scope_id, service, _r_opt) => {
                    if service.is_stopped() {
                        rt.remove_microservice(scope_id);
                    } else {
                        rt.upsert_microservice(scope_id, service);
                    }
                    if rt.microservices_stopped() {
                        break;
                    }
                }
            }
        }
        if let Some(root_handle) = self.link_to.take() {
            root_handle.shutdown().await
        }
        Ok(())
    }
}
