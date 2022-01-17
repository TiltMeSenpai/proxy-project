use std::{sync::atomic::{AtomicU32, Ordering}, pin::Pin, task::Poll, cell::RefCell};

use futures::lock::Mutex;
use futures_core::stream::Stream;
use hyper::{body::Bytes, Body};
use tokio::sync::broadcast::Sender;
use crate::proxy::{ProxyEvent, ProxyState};

#[derive(Clone, Debug)]
enum StreamFork {
    RequestStream(Sender<ProxyEvent>),
    ResponseStream(Sender<ProxyEvent>)
}

impl StreamFork {
    fn send_event(&self, id: u32, request_id: &AtomicU32, chunk: Bytes) {
        let request_id = request_id.fetch_add(1, Ordering::Relaxed);
        match self {
            Self::RequestStream(stream) => {
                stream.send(
                    ProxyEvent {
                        id,
                        event: ProxyState::RequestChunk {
                            id: request_id,
                            chunk
                        },
                    }
                ).unwrap();
            },
            Self::ResponseStream(stream) => {
                stream.send(
                    ProxyEvent {
                        id,
                        event: ProxyState::ResponseChunk {
                            id: request_id,
                            chunk
                        },
                    }
                ).unwrap();
            }
        }
    }

    fn close(&self, id: u32, request_id: &AtomicU32) {
        let request_id = request_id.fetch_add(1, Ordering::Relaxed);
        match self {
            Self::RequestStream(stream) => {
                stream.send(
                    ProxyEvent {
                        id,
                        event: ProxyState::RequestDone {
                            id: request_id,
                        },
                    }
                ).unwrap();
            },
            Self::ResponseStream(stream) => {
                stream.send(
                    ProxyEvent {
                        id,
                        event: ProxyState::ResponseDone {
                            id: request_id,
                        },
                    }
                ).unwrap();
            }
        }
    }
}

#[derive(Debug)]
pub struct InnerStreamBody {
    inner: Body,
    id: u32,
    request_id: AtomicU32,
    stream: StreamFork
}

#[repr(transparent)]
#[derive(Debug)]
pub struct StreamBody(Mutex<RefCell<Option<InnerStreamBody>>>);

impl StreamBody {
    fn new(inner: InnerStreamBody) -> Self{
        Self(Mutex::new(RefCell::new(Some(inner))))
    }

    fn try_take(&self) -> Option<InnerStreamBody> {
        self.0.try_lock().and_then(|lock| lock.take() )
    }
}

impl StreamBody {
    pub fn stream_request(inner: Body, id: u32, channel: Sender<ProxyEvent>) -> Self {
        Self::new( InnerStreamBody{
            inner,
            id,
            request_id: AtomicU32::new(0),
            stream: StreamFork::RequestStream(channel)
        })
    }

    pub fn stream_response(inner: Body, id: u32, channel: Sender<ProxyEvent>) -> Self {
        Self::new(InnerStreamBody {
            inner,
            id,
            request_id: AtomicU32::new(0),
            stream: StreamFork::ResponseStream(channel)
        })
    }

    pub fn try_into_body(&self) -> Option<Body> {
        self.try_take().map(Body::wrap_stream)
    }
}

impl Stream for InnerStreamBody {
    type Item = hyper::Result<Bytes>;

    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        let pinned_inner = Pin::new(&mut self.inner);

        match Stream::poll_next(pinned_inner, cx) {
            Poll::Ready(next) => Poll::Ready({
                match next {
                    None => {
                        self.stream.close(self.id, &self.request_id);
                        None
                    },
                    Some(n) => Some(match n {
                        Ok(chunk) => {
                            self.stream.send_event(self.id, &self.request_id, chunk.clone());
                            Ok(chunk)
                        },
                        Err(e) => Err(e)
                    })
                }
            }),
            Poll::Pending => Poll::Pending
        }
    }
}