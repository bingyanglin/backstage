use super::{Error, Event, Interface, Response, WebsocketSenderEvent};
use crate::core::*;
use futures::stream::{SplitStream, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

pub struct WebsocketReceiver {
    sender_handle: UnboundedHandle<WebsocketSenderEvent>,
    split_stream: Option<WsRx>,
}

impl WebsocketReceiver {
    /// Create new WebsocketReceiver struct
    pub fn new(split_stream: SplitStream<WebSocketStream<TcpStream>>, sender_handle: UnboundedHandle<WebsocketSenderEvent>) -> Self {
        Self {
            sender_handle,
            split_stream: Some(split_stream),
        }
    }
}

#[async_trait::async_trait]
impl ChannelBuilder<WsRxChannel> for WebsocketReceiver {
    async fn build_channel(&mut self) -> Result<WsRxChannel, Reason>
    where
        Self: Actor<Channel = WsRxChannel>,
    {
        if let Some(stream) = self.split_stream.take() {
            Ok(WsRxChannel(stream))
        } else {
            Err(Reason::Exit)
        }
    }
}

#[async_trait::async_trait]
impl Actor for WebsocketReceiver {
    type Channel = WsRxChannel;
    async fn init<S: Supervise<Self>>(&mut self, _rt: &mut Self::Context<S>) -> Result<Self::Data, Reason> {
        Ok(())
    }
    async fn run<S: Supervise<Self>>(&mut self, rt: &mut Self::Context<S>, _data: Self::Data) -> ActorResult {
        while let Some(Ok(message)) = rt.inbox_mut().next().await {
            // Deserialize message::text
            match message {
                Message::Text(text) => {
                    if let Ok(interface) = serde_json::from_str::<Interface>(&text) {
                        let mut targeted_scope_id_opt = interface.actor_path.clone().destination().await;
                        match interface.event {
                            Event::Shutdown => {
                                if let Some(scope_id) = targeted_scope_id_opt.take() {
                                    if let Err(err) = rt.shutdown_scope(scope_id).await {
                                        let err_string = err.to_string();
                                        let r = Error::Shutdown(interface.actor_path, err_string);
                                        self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                    } else {
                                        let r = Response::Shutdown(interface.actor_path);
                                        self.sender_handle.send(WebsocketSenderEvent::Result(Ok(r))).ok();
                                    };
                                } else {
                                    let err_string = "Unreachable ActorPath".to_string();
                                    let r = Error::Shutdown(interface.actor_path, err_string);
                                    self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                }
                            }
                            Event::RequestServiceTree => {
                                if let Some(scope_id) = targeted_scope_id_opt.take() {
                                    if let Some(service) = rt.lookup::<Service>(scope_id).await {
                                        let r = Response::ServiceTree(service);
                                        self.sender_handle.send(WebsocketSenderEvent::Result(Ok(r))).ok();
                                    } else {
                                        let r = Error::ServiceTree("Service not available".into());
                                        self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                    };
                                } else {
                                    let r = Error::ServiceTree("Unreachable ActorPath".into());
                                    self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                }
                            }
                            Event::Cast(message_to_route) => {
                                if let Some(scope_id) = targeted_scope_id_opt.take() {
                                    let route_message = message_to_route.clone();
                                    match rt.send(scope_id, message_to_route).await {
                                        Ok(()) => {
                                            let r = Response::Sent(route_message);
                                            self.sender_handle.send(WebsocketSenderEvent::Result(Ok(r))).ok();
                                        }
                                        Err(e) => {
                                            let err = format!("{}", e);
                                            let r = Error::Cast(interface.actor_path, route_message, err);
                                            self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                        }
                                    };
                                } else {
                                    let r = Error::Cast(interface.actor_path, message_to_route, "Unreachable ActorPath".into());
                                    self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                }
                            }
                            Event::Call(message_to_route_with_responder) => {
                                // todo
                            }
                        }
                    };
                }
                Message::Close(_) => {
                    break;
                }
                _ => {}
            }
        }
        self.sender_handle.shutdown().await;
        Ok(())
    }
}
