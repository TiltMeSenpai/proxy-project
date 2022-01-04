use std::convert::Infallible;
use std::io::{Error, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;

use hyper::http::uri::{Parts, Authority, Scheme};
use hyper::server::conn::{AddrStream, Http};
use hyper::upgrade::Upgraded;
use hyper::{Request, Body, Response, Server, Client, Method, StatusCode, Uri};
use hyper::service::{make_service_fn, service_fn};

use openssl::asn1::Asn1Integer;
use openssl::bn::MsbOption;
use openssl::conf::Conf;
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::x509::{X509NameBuilder, X509Builder, X509Extension, extension};
use rustls::server::{WantsServerCert, ResolvesServerCert};
use rustls::sign::RsaSigningKey;
use rustls::{ServerConfig, ConfigBuilder, Certificate};
use tokio::io::DuplexStream;
use tokio::sync::mpsc::{Sender, Receiver};
use tokio_rustls::TlsAcceptor;
use openssl::{x509::X509, asn1::Asn1Time, bn::BigNum};
use lazy_static::lazy_static;

lazy_static!{
    static ref SSL_CONF: Conf = Conf::new(openssl::conf::ConfMethod::default()).unwrap();
    static ref KEYPAIR: (X509, PKey<Private>) = cert().unwrap();
}

struct TlsProxy {
    host: Uri
}

fn cert() -> Option<(X509, PKey<Private>)>{
    use std::fs::File;
    if let (Ok(mut cert_file), Ok(mut key_file)) = (File::open("data/cert"), File::open("data/key")) {
        let mut cert: Vec<u8> = Vec::new();
        cert_file.read_to_end(&mut cert).unwrap();
        let mut key: Vec<u8> = Vec::new();
        key_file.read_to_end(&mut key).unwrap();
        Some((X509::from_pem(&cert[..]).unwrap(), PKey::private_key_from_der(&key[..]).unwrap()))
    } else {
        let mut cert = X509Builder::new().ok()?;
        cert.set_version(2).ok()?;
        cert.set_not_before(Asn1Time::days_from_now(0).ok()?.as_ref()).ok()?;
        cert.set_not_after(Asn1Time::days_from_now(365).ok()?.as_ref()).ok()?;
        let mut name = X509NameBuilder::new().ok()?;
        name.append_entry_by_text("CN", "localhost").ok()?;
        name.append_entry_by_text("O", "proxy").ok()?;
        name.append_entry_by_text("OU", "Root CA").ok()?;
        let name = name.build();
        cert.set_issuer_name(&name).unwrap();
        cert.set_subject_name(&name).unwrap();
        let key = PKey::from_rsa(Rsa::generate(2048).ok()?).ok()?;
        cert.append_extension(extension::KeyUsage::new().critical().digital_signature().key_cert_sign().build().ok()?).unwrap();
        cert.append_extension(extension::ExtendedKeyUsage::new().server_auth().build().ok()?).unwrap();
        {   
            let ctx = cert.x509v3_context(None, Some(&SSL_CONF));
            // Rust gets angy abut this but there's not much we can do about it
            #[allow(mutable_borrow_reservation_conflict)]
            cert.append_extension(X509Extension::new(Some(&SSL_CONF), Some(&ctx), "subjectKeyIdentifier", "hash").ok()?).unwrap();
        }
        cert.set_pubkey(&key).unwrap();
        cert.sign(&key, MessageDigest::sha512()).ok()?;
        let cert = cert.build();

        let mut cert_file = File::create("data/cert").ok()?;
        cert_file.write(&cert.to_pem().ok()?[..]).ok()?;
        let mut key_file = File::create("data/key").ok()?;
        key_file.write(&key.private_key_to_der().ok()?[..]).ok()?;
        Some((cert, key))
    }
}

impl ResolvesServerCert for TlsProxy {
    fn resolve(&self, client_hello: rustls::server::ClientHello) -> Option<Arc<rustls::sign::CertifiedKey>> {
        let (pubkey, privkey): &(X509, PKey<Private>) = &KEYPAIR;
        let hostname = client_hello.server_name().unwrap_or(self.host.host().unwrap());
        println!("Negotiating TLS to {}", hostname);
        if let Ok(mut cert) = X509::builder() {
                let mut serial = BigNum::new().unwrap();
                if let Err(e) = serial.rand(128, MsbOption::MAYBE_ZERO, true) {
                    eprintln!("Error generating cert serial: {}", e);
                    return None
                }
                let serial = Asn1Integer::from_bn(&serial).unwrap();

                cert.set_not_before(&Asn1Time::days_from_now(0).unwrap()).unwrap();
                cert.set_not_after(&Asn1Time::days_from_now(365).unwrap()).unwrap();
                cert.set_version(2).unwrap();
                cert.set_serial_number(&serial).unwrap();

                let mut x509_name = X509NameBuilder::new().unwrap();
                x509_name.append_entry_by_text("CN", hostname).unwrap();
                let x509_name = x509_name.build();
                cert.set_issuer_name(pubkey.subject_name()).unwrap();
                cert.set_subject_name(&x509_name).unwrap();
                cert.append_extension(extension::ExtendedKeyUsage::new().critical().server_auth().build().unwrap()).unwrap();
                {   
                    let ctx = cert.x509v3_context(Some(&pubkey), Some(&SSL_CONF));
                    // Rust gets angy abut this but there's not much we can do about it
                    #[allow(mutable_borrow_reservation_conflict)]
                    cert.append_extension(extension::SubjectAlternativeName::new().critical().uri(hostname).build(&ctx).unwrap()).unwrap();
                }
                cert.set_pubkey(&privkey).unwrap();
                cert.sign(&privkey, MessageDigest::sha512()).unwrap();
                let cert = cert.build();
                let privkey = rustls::PrivateKey(privkey.private_key_to_der().unwrap());
                Some(Arc::new(
                    rustls::sign::CertifiedKey {
                        cert: vec![rustls::Certificate(cert.to_der().unwrap()), rustls::Certificate(pubkey.to_der().unwrap())],
                        key: Arc::new(RsaSigningKey::new(&privkey).unwrap()),
                        ocsp: None,
                        sct_list: None
                    }
                ))
        } else {
            None
        }
    }
}

fn server_config_for(host: &Uri) -> Arc<ServerConfig>{
    Arc::new(rustls::ServerConfig::builder()
        .with_safe_default_cipher_suites()
        .with_safe_default_kx_groups()
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(TlsProxy {
            host: host.clone()
        })))
}

async fn handle_tls(upgraded: Upgraded, host: &Uri) -> Result<(), Error> {
    let acceptor = TlsAcceptor::from(server_config_for(host));
    let stream = acceptor.accept(upgraded).await.unwrap();
    let host = String::from(stream.get_ref().1.sni_hostname().unwrap_or(host.host().unwrap()));
    if let Err(e) = Http::new()
        .serve_connection(stream, service_fn(move |mut req| {
            let mut uri = req.uri().to_owned().into_parts();
            uri.authority = Some(Authority::from_maybe_shared(host.clone()).unwrap());
            uri.scheme = Some(Scheme::HTTPS);
            *req.uri_mut() = Uri::from_parts(uri).unwrap();
            handle(req)
        })).await {
            eprintln!("TLS Proxy Error: {}", e)
        }
    Ok(())
}

async fn handle(req: Request<Body>) -> Result<Response<Body>, Infallible>{
    println!("Got request {:?} {} {}", req.uri().scheme(), req.method(), req.uri());
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
                    Err(e) => eprintln!("CONNECT Error: {}", e)
                }
            });
            let resp = Response::new(Body::empty()); // Empty response, CONNECT expects a 200
            Ok(resp)
        }
        _default => handle(req).await
    }
}

#[tokio::main]
async fn main() {
    let addr = SocketAddr::from(([0, 0, 0, 0], 1337));

    let make_svc = make_service_fn(|socket: &AddrStream| {
        println!("Connection from {}", socket.remote_addr());
        async move {
            Ok::<_, Infallible>(service_fn(handle_with_upgrade))
        }
    });

    let server = Server::bind(&addr).serve(make_svc);

    if let Err(e) = server.await {
        eprintln!("Server error: {}", e);
    }

}
