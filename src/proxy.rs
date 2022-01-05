#![allow(dead_code)]

use std::convert::Infallible;
use std::future::Future;
use std::io::Error;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use hyper::http::uri::{Authority, Scheme};
use hyper::server::conn::{Http, AddrStream};
use hyper::service::{service_fn, make_service_fn};
use hyper::upgrade::Upgraded;
use hyper::{Request, Response, Body, Method, Uri, Client, Server};
use tokio::task::JoinHandle;
use tokio::sync::mpsc::{Sender,Receiver, channel};
use tokio_rustls::TlsAcceptor;
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
    pub host: [u8; 4],
    pub port: u16
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            pubkey_path: "data/cert".to_string(),
            privkey_path: "data/key".to_string(),
            host: [0, 0, 0, 0],
            port: 1337
        }
    }
}

impl ProxyConfig {
    pub fn build(self) -> Arc<Proxy>{
        Proxy::new(self)
    }
}

pub struct Proxy {
    host: [u8; 4],
    port: u16, 
    cert_store: Arc<CertStore>,
    request_sink: Receiver<ProxyEvent>,
    request_source: Sender<ProxyEvent>,
    id: Arc<AtomicU32>,
}

impl Proxy {
    fn new(config: ProxyConfig) -> Arc<Self> {
        let(tx, rx) = channel(16);
        Arc::new(Self {
            host: config.host,
            port: config.port,
            cert_store: Arc::new(
                CertStore::load_or_create(&config.pubkey_path, &config.privkey_path)
            ),
            request_source: tx,
            request_sink: rx,
            id: Arc::new(AtomicU32::new(0))
        })
    }

    pub fn run(proxy: Arc<Self>) -> JoinHandle<Result<(), hyper::Error>>{
        let proxy_move = proxy.clone();
        let make_svc = make_service_fn(move |_socket: &AddrStream| {
            let proxy = proxy_move.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let proxy = proxy.clone();
                    Self::handle_with_upgrade(proxy, req)
                }))
            }
        });
        let addr = SocketAddr::from((proxy.host.to_owned(), proxy.port));

        tokio::spawn(Server::bind(&addr).serve(make_svc))
    }

    async fn wrap_tls(proxy: Arc<Self>, upgraded: Upgraded, host: &Uri) -> Result<(), Error> {
        let proxy = proxy.clone();
        let acceptor = TlsAcceptor::from(
            Arc::new(
                rustls::ServerConfig::builder()
                    .with_safe_default_cipher_suites()
                    .with_safe_default_kx_groups()
                    .with_safe_default_protocol_versions()
                    .unwrap()
                    .with_no_client_auth()
                    .with_cert_resolver(CertStore::build_cert(
                        &proxy.cert_store,
                        host.host().map(|host| host.to_owned()),
                    )),
            )
        );
        let stream = acceptor.accept(upgraded).await.unwrap();
        let host = String::from(
            stream
                .get_ref()
                .1
                .sni_hostname()
                .unwrap_or(host.host().unwrap()),
        );
        if let Err(e) = Http::new()
            .serve_connection(
                stream,
                service_fn(move |mut req| {
                    let mut uri = req.uri().to_owned().into_parts();
                    uri.authority = Some(Authority::from_maybe_shared(host.clone()).unwrap());
                    uri.scheme = Some(Scheme::HTTPS);
                    *req.uri_mut() = Uri::from_parts(uri).unwrap();
                    Self::handle_req(req, proxy.request_source.clone(), proxy.id.clone())
                }),
            )
            .await
        {
            eprintln!("TLS Proxy Error: {}", e)
        }
        Ok(())
    }

    async fn handle_with_upgrade(proxy: Arc<Self>, mut req: Request<Body>) -> Result<Response<Body>, Infallible> {
        match req.method() {
            &Method::CONNECT => {
                tokio::spawn(async move {
                    let proxy = proxy.clone();
                    match hyper::upgrade::on(&mut req).await {
                        Ok(upgraded) => {
                            if let Err(e) = Self::wrap_tls(proxy, upgraded, req.uri()).await {
                                eprintln!("TLS proxy error: {}", e)
                            }
                        }
                        Err(e) => eprintln!("CONNECT Error: {}", e),
                    }
                });
                let resp = Response::new(Body::empty()); // Empty response, CONNECT expects a 200
                Ok(resp)
            }
            _default => Self::handle_req(req, proxy.request_source.clone(), proxy.id.clone()).await,
        }
    }

    async fn handle_req(req: Request<Body>, channel: Sender<ProxyEvent>, id: Arc<AtomicU32>) -> Result<Response<Body>, Infallible> {
        let id = id.fetch_add(1, Ordering::Relaxed);
        println!("{} Got request {:?}", id, req);
        let client = Client::builder().build(hyper_tls::HttpsConnector::new());
        let resp = client.request(req).await.unwrap();
        println!("{} Got response {:?}", id, resp);
        Ok(resp)
    }
}