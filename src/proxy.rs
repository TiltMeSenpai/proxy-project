#![allow(dead_code)]

use std::convert::Infallible;
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::Poll;

use hyper::http::uri::{Authority, Scheme};
use hyper::server::conn::{Http, AddrStream};
use hyper::service::Service;
use hyper::upgrade::{Upgraded, self};
use hyper::{Request, Response, Body, Method, Uri, Client, Server};
use rustls::ServerConfig;
use tokio::task::JoinHandle;
use tokio::sync::mpsc::{Sender,Receiver, channel};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use crate::cert::{CertStore};


#[derive(Debug)]
enum ProxyState {
    Request(Request<Body>),
    Response(Response<Body>),
    Pair(Request<Body>, Response<Body>)
}

#[derive(Debug)]
struct ProxyEvent {
    id: u32,
    event: ProxyState
}

pub struct ProxyConfig {
    pub pubkey_path: String,
    pub privkey_path: String,
    pub listen: SocketAddr,
    pub starting_id: u32
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            pubkey_path: "data/cert".to_string(),
            privkey_path: "data/key".to_string(),
            listen: SocketAddr::from((
                [0, 0, 0, 0],
                1337
            )),
            starting_id: 0
        }
    }
}

impl ProxyConfig {
    pub fn build(self) -> ProxyServer {
        ProxyServer::new(self)
    }
}

pub struct ProxyServer {
    listen: SocketAddr,
    request_sink: Receiver<ProxyEvent>,
    core: ProxyCore,
}

impl ProxyServer {
    pub fn new(conf: ProxyConfig) -> Self {
        let (tx, rx) = channel(64);
        Self {
            listen: conf.listen,
            request_sink: rx,
            core: ProxyCore {
                cert_store: Arc::new(CertStore::load_or_create(&conf.pubkey_path, &conf.privkey_path)),
                request_source: Arc::new(tx),
                id: Arc::new(AtomicU32::new(conf.starting_id)),
                fallback_host: None,
                client: Client::builder().build(hyper_tls::HttpsConnector::new())
            }
        }
    }

    pub fn run(self) -> JoinHandle<Result<(), hyper::Error>>{
        tokio::spawn (
            Server::bind(&self.listen)
            .serve(self)
        )
    }
}

impl<'a> Service<&'a AddrStream> for ProxyServer {
    type Response = ProxyCore;

    type Error = Infallible;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: &'a AddrStream) -> Self::Future {
        let core = self.core.clone();
        let fut = async move {
            Ok(core)
        };
        Box::pin(fut)
    }
}

#[derive(Clone)]
pub struct ProxyCore {
    cert_store: Arc<CertStore>,
    request_source: Arc<Sender<ProxyEvent>>,
    id: Arc<AtomicU32>,
    fallback_host: Option<String>,
    client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>, Body>
}

impl Service<Request<Body>> for ProxyCore {
    type Response = Response<Body>;

    type Error = Error;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let proxy = self.clone();
        let host = req.uri().host().map(String::from);
        Box::pin(async move {
            if let &Method::CONNECT = req.method()
            {
                tokio::spawn(async move {
                    match upgrade::on(req).await {
                        Ok(upgraded) => {
                            proxy.do_tls_upgrade(upgraded, host).await;
                        },
                        Err(e) => {
                            eprint!("Error upgrading connection: {}", e);
                        }
                    }
                });
                Ok(Response::default())
            }
            else {
                if let Some(host) = host.or(proxy.fallback_host){
                    let id = proxy.id.fetch_add(1, Ordering::Relaxed);
                    let mut uri = req.uri().to_owned().into_parts();
                    uri.authority = Some(Authority::from_maybe_shared(host.clone()).unwrap());
                    uri.scheme = Some(Scheme::HTTPS);
                    *req.uri_mut() = Uri::from_parts(uri).unwrap();
                    println!("Request: {} {:?}", id, req);
                    Ok(proxy.client.request(req).await.unwrap())
                } else {
                    Err(Error::new(ErrorKind::InvalidInput ,"No host requested and no host derived from CONNECT or SNI"))
                }
            }
        })
    }
}

impl ProxyCore {
    async fn do_tls_upgrade(&self, conn: Upgraded, fallback_host: Option<String>) {
        let conf = Arc::new(ServerConfig::builder()
            .with_safe_default_cipher_suites()
            .with_safe_default_kx_groups()
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(CertStore::build_cert(&self.cert_store, fallback_host.to_owned())));
        let accepted = TlsAcceptor::from(conf).accept(conn).await.unwrap();
        println!("TLS connected with SNI {:?}", accepted.get_ref().1.sni_hostname());
        let mut service = self.clone();
        service.fallback_host = Self::get_host(&accepted, &fallback_host);
        if let Err(e) = Http::new().serve_connection(accepted, service).await
            {
                eprintln!("Error serving wrapped connection: {}", e)
            }
    }

    fn get_host(conn: &TlsStream<Upgraded>, fallback_host: &Option<String>) -> Option<String> {
        conn.get_ref().1.sni_hostname().map(String::from).or(fallback_host.to_owned())
    }
}