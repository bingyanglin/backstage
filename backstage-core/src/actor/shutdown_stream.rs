use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{
    future::{self, FusedFuture},
    stream::{self, FusedStream},
    FutureExt, Stream, StreamExt,
};
use tokio::sync::oneshot;

type Shutdown = oneshot::Receiver<()>;
type FusedShutdown = future::Fuse<Shutdown>;

/// A stream with a shutdown.
///
/// This type wraps a shutdown receiver and a stream to produce a new stream that ends when the
/// shutdown receiver is triggered or when the stream ends.
pub struct ShutdownStream<S> {
    shutdown: FusedShutdown,
    stream: stream::Fuse<S>,
}

impl<S: Stream> ShutdownStream<S> {
    /// Create a new `ShutdownStream` from a shutdown receiver and an unfused stream.
    ///
    /// This method receives the stream to be wrapped and a `oneshot::Receiver` for the shutdown.
    /// Both the stream and the shutdown receiver are fused to avoid polling already completed
    /// futures.
    pub fn new(shutdown: Shutdown, stream: S) -> Self {
        Self {
            shutdown: shutdown.fuse(),
            stream: stream.fuse(),
        }
    }

    /// Create a new `ShutdownStream` from a fused shutdown receiver and a fused stream.
    ///
    /// This method receives the fused stream to be wrapped and a fused `oneshot::Receiver` for the shutdown.
    #[allow(dead_code)]
    pub fn from_fused(shutdown: FusedShutdown, stream: stream::Fuse<S>) -> Self {
        Self { shutdown, stream }
    }

    /// Consume and split the `ShutdownStream` into its shutdown receiver and stream.
    #[allow(dead_code)]
    pub fn split(self) -> (FusedShutdown, stream::Fuse<S>) {
        (self.shutdown, self.stream)
    }
}

impl<S: Stream<Item = T> + Unpin, T> Stream for ShutdownStream<S> {
    type Item = T;
    /// The shutdown receiver is polled first, if it is not ready, the stream is polled. This
    /// guarantees that checking for shutdown always happens first.
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        if !self.shutdown.is_terminated() {
            if self.shutdown.poll_unpin(cx).is_ready() {
                return Poll::Ready(None);
            }

            if !self.stream.is_terminated() {
                return self.stream.poll_next_unpin(cx);
            }
        }

        Poll::Ready(None)
    }
}

impl<S: Stream<Item = T> + Unpin, T> FusedStream for ShutdownStream<S> {
    fn is_terminated(&self) -> bool {
        self.shutdown.is_terminated() || self.stream.is_terminated()
    }
}
