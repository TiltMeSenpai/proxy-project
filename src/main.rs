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

fn server_config_for(host: &Uri) -> Arc<ServerConfig> {
    Arc::new(
        rustls::ServerConfig::builder()
            .with_safe_default_cipher_suites()
            .with_safe_default_kx_groups()
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(cert::CertStore::build_cert(
                &cert::CERT_STORE,
                host.host().map(|host| host.to_owned()),
            )),
    )
}

async fn handle_tls(upgraded: Upgraded, host: &Uri) -> Result<(), Error> {
    let acceptor = TlsAcceptor::from(server_config_for(host));
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
                handle(req)
            }),
        )
        .await
    {
        eprintln!("TLS Proxy Error: {}", e)
    }
    Ok(())
}

async fn handle(req: Request<Body>) -> Result<Response<Body>, Infallible> {
    println!(
        "Got request {:?} {} {}",
        req.uri().scheme(),
        req.method(),
        req.uri()
    );
    let client = Client::builder().build(hyper_tls::HttpsConnector::new());
    let resp = client.request(req).await.unwrap();
    println!("Got response {:?}", resp);
    Ok(resp)
}

async fn handle_with_upgrade(mut req: Request<Body>) -> Result<Response<Body>, Infallible> {
    match req.method() {
        &Method::CONNECT => {
            tokio::spawn(async move {
                match hyper::upgrade::on(&mut req).await {
                    Ok(upgraded) => {
                        if let Err(e) = handle_tls(upgraded, req.uri()).await {
                            eprintln!("TLS proxy error: {}", e)
                        }
                    }
                    Err(e) => eprintln!("CONNECT Error: {}", e),
                }
            });
            let resp = Response::new(Body::empty()); // Empty response, CONNECT expects a 200
            Ok(resp)
        }
        _default => handle(req).await,
    }
}

#[tokio::main]
async fn main() {
    let addr = SocketAddr::from(([0, 0, 0, 0], 1337));

    let make_svc = make_service_fn(|socket: &AddrStream| {
        println!("Connection from {}", socket.remote_addr());
        async move { Ok::<_, Infallible>(service_fn(handle_with_upgrade)) }
    });

    let server = Server::bind(&addr).serve(make_svc);

    if let Err(e) = server.await {
        eprintln!("Server error: {}", e);
    }
}
