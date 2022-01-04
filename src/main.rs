use std::convert::Infallible;
use std::io::Error;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::http::uri::{Authority, Scheme};
use hyper::server::conn::{AddrStream, Http};
use hyper::service::{make_service_fn, service_fn};
use hyper::upgrade::Upgraded;
use hyper::{Body, Client, Method, Request, Response, Server, Uri};

use rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

mod cert;
mod proxy;

#[tokio::main]
async fn main() {
    let config: proxy::ProxyConfig = Default::default();
    let proxy = config.build();
    let _ = proxy::Proxy::run(proxy).await.unwrap();
}
