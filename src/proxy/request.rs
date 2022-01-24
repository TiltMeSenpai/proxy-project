use hyper::{http::{Method, Uri, Version, HeaderMap, HeaderValue}, Body, upgrade::OnUpgrade};
use crate::proxy::body::StreamBody;

use super::{ProxyEvent, Sender, ProxyState};

#[derive(Clone, Debug, PartialEq)]
pub struct RequestHead {
    pub method: Method,
    pub uri: Uri,
    pub version: Version,
    pub headers: HeaderMap<HeaderValue>,
}

#[derive(Debug)]
pub struct Request {
    pub head: RequestHead,
    pub body: StreamBody,
}

impl Request {
    pub async fn from_request(req: hyper::Request<Body>, id: u32, channel: Sender<ProxyEvent>) -> (Self, Option<OnUpgrade>) {
        let (mut parts, body) = req.into_parts();
        let head = RequestHead {
                method:  parts.method,
                uri:     parts.uri,
                version: parts.version,
                headers: parts.headers,
        };
        let (event, completion) = ProxyEvent::req_head(id, &head);
        channel.send(event).await.unwrap();
        let head = match completion.await {
            Ok(ProxyState::RequestHead(head)) => head,
            Ok(e) => {
                println!("Got unexpected result {:?}", e);
                head
            }
            Err(_) => {
                println!("Dropped sender");
                head
            }
        };
        (Self {
            head,
            body: StreamBody::stream_request(body, id,  channel),
        },
        parts.extensions.remove()
        )
    }
}

impl Into<hyper::Request<Body>> for Request {
    fn into(self) -> hyper::Request<Body> {
        let req = hyper::Request::builder()
            .method(self.head.method)
            .uri(self.head.uri)
            .version(self.head.version);
        let req = self.head.headers.iter().fold(
            req,
            | req, (name, item) | req.header(name, item)
        );
        req
            .body(self.body.try_into_body().expect("Body was previously consumed"))
            .unwrap()
    }
}