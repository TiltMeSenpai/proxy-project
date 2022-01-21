use std::{pin::Pin, task::Poll, cell::RefCell};

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
    fn send_event(&self, id: u32, chunk: Bytes) {
        match self {
            Self::RequestStream(stream) => {
                stream.send(
                    ProxyEvent {
                        id,
                        event: ProxyState::RequestChunk {
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
                            chunk
                        },
                    }
                ).unwrap();
            }
        }
    }

    fn close(&self, id: u32) {
        match self {
            Self::RequestStream(stream) => {
                stream.send(
                    ProxyEvent {
                        id,
                        event: ProxyState::RequestDone
                    }
                ).unwrap();
            },
            Self::ResponseStream(stream) => {
                stream.send(
                    ProxyEvent {
                        id,
                        event: ProxyState::ResponseDone
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
            stream: StreamFork::RequestStream(channel)
        })
    }

    pub fn stream_response(inner: Body, id: u32, channel: Sender<ProxyEvent>) -> Self {
        Self::new(InnerStreamBody {
            inner,
            id,
            stream: StreamFork::ResponseStream(channel)
        })
    }

    pub fn try_into_body(&self) -> Option<Body> {
        self.try_take().map(Body::wrap_stream)
    }
}

impl Drop for InnerStreamBody {
    fn drop(&mut self) {
        self.stream.close(self.id)
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
                        None
                    },
                    Some(n) => Some(match n {
                        Ok(chunk) => {
                            self.stream.send_event(self.id, chunk.clone());
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