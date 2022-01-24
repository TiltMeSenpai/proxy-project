use std::{pin::Pin, task::Poll, cell::RefCell};

use futures::{lock::Mutex, Future, StreamExt};
use futures_core::stream::Stream;
use hyper::{body::Bytes, Body};
use tokio::sync::mpsc::Sender;
use crate::proxy::{ProxyEvent, ProxyState};

#[derive(Clone, Debug)]
enum StreamFork {
    RequestStream(Sender<ProxyEvent>),
    ResponseStream(Sender<ProxyEvent>)
}

impl StreamFork {
    async fn send_event(&self, id: u32, chunk: Bytes) -> Bytes {
        match self {
            Self::RequestStream(stream) => {
                let (event, completion) = ProxyEvent::req_chunk(id, &chunk);
                stream.send( event ).await.unwrap();
                match completion.await {
                    Ok(ProxyState::RequestChunk(chunk)) => chunk,
                    Ok(e) => {
                        println!("Got unexpected response: {:?}", e);
                        chunk
                    }
                    Err(_) => chunk
                }
            },
            Self::ResponseStream(stream) => {
                let (event, completion) = ProxyEvent::resp_chunk(id, &chunk);
                stream.send( event ).await.unwrap();
                match completion.await {
                    Ok(ProxyState::ResponseChunk(chunk)) => chunk,
                    Ok(e) => {
                        println!("Got unexpected response: {:?}", e);
                        chunk
                    }
                    Err(_) => chunk
                }
            }
        }
    }

    fn close(&self, id: u32) {
        match self {
            Self::RequestStream(stream) => {
                stream.try_send(
                    ProxyEvent::req_done(id)
                ).unwrap();
            },
            Self::ResponseStream(stream) => {
                stream.try_send(
                    ProxyEvent::req_done(id)
                ).unwrap();
            }
        }
    }
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
            stream: StreamFork::RequestStream(channel),
        })
    }

    pub fn stream_response(inner: Body, id: u32, channel: Sender<ProxyEvent>) -> Self {
        Self::new(InnerStreamBody {
            inner,
            id,
            stream: StreamFork::ResponseStream(channel),
        })
    }

    pub fn try_into_body(&self) -> Option<Body> {
        self.try_take().map(|inner| Body::wrap_stream(inner.to_stream()))
    }
}

pub struct InnerStreamBody {
    inner: Body,
    id: u32,
    stream: StreamFork
}

impl Drop for InnerStreamBody {
    fn drop(&mut self) {
        self.stream.close(self.id)
    }
}


impl InnerStreamBody {
    fn to_stream(self) -> impl Stream<Item = hyper::Result<Bytes>>
    {
        futures::stream::unfold(self, | mut stream | async move {
            match stream.inner.next().await {
                Some(Ok(next)) => {
                    let bytes = stream.stream.send_event(stream.id, next).await;
                    Some((Ok(bytes), stream))
                },
                Some(Err(e)) => Some((Err(e), stream)),
                None => None
            }
        })
    }
}