use hyper::{http::{Version, HeaderMap, HeaderValue}, Body, StatusCode, upgrade::OnUpgrade};
use crate::proxy::body::StreamBody;

use super::{ProxyEvent, Sender, ProxyState};

#[derive(Clone, Debug, PartialEq)]
pub struct ResponseHead {
    pub status: StatusCode,
    pub version: Version,
    pub headers: HeaderMap<HeaderValue>,
}

#[derive(Debug)]
pub struct Response {
    pub head: ResponseHead,
    pub body: StreamBody
}

impl Response {
    pub async fn from_response(resp: hyper::Response<Body>, id: u32, channel: Sender<ProxyEvent>) -> (Self, Option<OnUpgrade>) {
        let (mut parts, body) = resp.into_parts();
        let head = ResponseHead {
                status:  parts.status,
                version: parts.version,
                headers: parts.headers,
        };
        let (event, completion) = ProxyEvent::resp_head(id, &head);
        channel.send(event).await.unwrap();
        let head = match completion.await {
            Ok(ProxyState::ResponseHead(head)) => head,
            Ok(e) => {
                println!("Got unexpected result {:?}", e);
                head
            }
            Err(_) => head
        };
        (Self {
            head,
            body: StreamBody::stream_response(body, id,  channel)
        },
        parts.extensions.remove()
        )
    }
}

impl Into<hyper::Response<Body>> for Response {
    fn into(self) -> hyper::Response<Body> {
        let resp = hyper::Response::builder()
            .status(self.head.status)
            .version(self.head.version);
        let resp = self.head.headers.iter().fold(
            resp,
            | req, (name, item) | req.header(name, item)
        );
        resp
            .body(self.body.try_into_body().expect("Body was previously consumed"))
            .unwrap()
    }
}