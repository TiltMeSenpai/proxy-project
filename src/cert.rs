use std::{
    fs::File,
    io::{Read, Write},
    sync::Arc,
};

use openssl::{
    asn1::{Asn1Integer, Asn1Time},
    bn::{BigNum, MsbOption},
    conf::Conf,
    hash::MessageDigest,
    pkey::{PKey, Private},
    rsa::Rsa,
    x509::{extension, X509Builder, X509Extension, X509NameBuilder, X509},
};

use lazy_static::lazy_static;
use rustls::{server::ResolvesServerCert, sign::RsaSigningKey};

lazy_static! {
    pub static ref SSL_CONF: Conf = Conf::new(openssl::conf::ConfMethod::default()).unwrap();
}

pub struct CertStore {
    privkey: PKey<Private>,
    pubkey: X509,
}

impl CertStore {
    pub fn load_or_create(pubkey_path: &str, privkey_path: &str) -> Self {
        Self::try_load_or_create(pubkey_path, privkey_path)
            .expect("Unable to load or create cert store")
    }

    pub fn try_load_or_create(pubkey_path: &str, privkey_path: &str) -> Option<Self> {
        CertStore::try_load(pubkey_path, privkey_path)
            .or_else(|| { CertStore::try_new(pubkey_path, privkey_path) })
    }

    pub fn try_new(pubkey_path: &str, privkey_path: &str) -> Option<Self> {
        println!("Creating new cert");
        let mut cert = X509Builder::new().ok()?;
        cert.set_version(2).ok()?;
        cert.set_not_before(Asn1Time::days_from_now(0).ok()?.as_ref())
            .ok()?;
        cert.set_not_after(Asn1Time::days_from_now(365).ok()?.as_ref())
            .ok()?;
        let mut name = X509NameBuilder::new().ok()?;
        name.append_entry_by_text("CN", "localhost").ok()?;
        name.append_entry_by_text("O", "proxy").ok()?;
        let name = name.build();
        cert.set_issuer_name(&name).unwrap();
        cert.set_subject_name(&name).unwrap();
        let key = PKey::from_rsa(Rsa::generate(2048).ok()?).ok()?;
        cert.append_extension(
            extension::KeyUsage::new()
                .critical()
                .digital_signature()
                .key_cert_sign()
                .build()
                .ok()?,
        )
        .unwrap();
        cert.append_extension(
            extension::ExtendedKeyUsage::new()
                .server_auth()
                .build()
                .ok()?,
        )
        .unwrap();
        cert.append_extension(
            extension::BasicConstraints::new()
                .critical()
                .ca()
                .build()
                .ok()?,
        )
        .unwrap();
        {
            let ctx = cert.x509v3_context(None, Some(&SSL_CONF));
            // Rust gets angy abut this but there's not much we can do about it
            #[allow(mutable_borrow_reservation_conflict)]
            cert.append_extension(
                X509Extension::new(Some(&SSL_CONF), Some(&ctx), "subjectKeyIdentifier", "hash")
                    .ok()?,
            )
            .unwrap();
        }
        cert.set_pubkey(&key).unwrap();
        cert.sign(&key, MessageDigest::sha512()).ok()?;
        let cert = cert.build();

        let mut cert_file = File::create(pubkey_path).ok()?;
        cert_file.write(&cert.to_pem().ok()?[..]).ok()?;
        let mut key_file = File::create(privkey_path).ok()?;
        key_file.write(&key.private_key_to_der().ok()?[..]).ok()?;
        Some(CertStore {
            privkey: key,
            pubkey: cert,
        })
    }

    fn try_load(pubkey_path: &str, privkey_path: &str) -> Option<Self> {
        let mut cert_file: File = File::open(pubkey_path).ok()?;
        let mut key_file: File = File::open(privkey_path).ok()?;
        let mut cert: Vec<u8> = Vec::new();
        cert_file.read_to_end(&mut cert).unwrap();
        let mut key: Vec<u8> = Vec::new();
        key_file.read_to_end(&mut key).unwrap();
        Some(Self {
            pubkey: X509::from_pem(&cert[..]).unwrap(),
            privkey: PKey::private_key_from_der(&key[..]).unwrap(),
        })
    }

    #[allow(dead_code)]
    pub fn build_cert(store: &Arc<Self>, hostname: Option<String>) -> Arc<CertResolver> {
        Arc::new(CertResolver {
            cert_store: store.to_owned(),
            fallback_host: hostname,
        })
    }
}

pub struct CertResolver {
    cert_store: Arc<CertStore>,
    fallback_host: Option<String>,
}

impl ResolvesServerCert for CertResolver {
    fn resolve(
        &self,
        client_hello: rustls::server::ClientHello,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        let privkey = &self.cert_store.privkey;
        let pubkey = &self.cert_store.pubkey;
        let hostname = client_hello
            .server_name()
            .map(|host| host.to_owned())
            .or(self.fallback_host.to_owned())?;

        println!("Negotiating TLS to {}", hostname);
        if let Ok(mut cert) = X509::builder() {
            let mut serial = BigNum::new().unwrap();
            if let Err(e) = serial.rand(128, MsbOption::MAYBE_ZERO, true) {
                eprintln!("Error generating cert serial: {}", e);
                return None;
            }
            let serial = Asn1Integer::from_bn(&serial).unwrap();

            cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
                .unwrap();
            cert.set_not_after(&Asn1Time::days_from_now(365).unwrap())
                .unwrap();
            cert.set_version(2).unwrap();
            cert.set_serial_number(&serial).unwrap();

            let mut x509_name = X509NameBuilder::new().unwrap();
            x509_name.append_entry_by_text("CN", &hostname).unwrap();
            let x509_name = x509_name.build();
            cert.set_issuer_name(pubkey.subject_name()).unwrap();
            cert.set_subject_name(&x509_name).unwrap();
            cert.append_extension(
                extension::ExtendedKeyUsage::new()
                    .critical()
                    .server_auth()
                    .build()
                    .unwrap(),
            )
            .unwrap();
            {
                let ctx = cert.x509v3_context(Some(&pubkey), Some(&SSL_CONF));
                // Rust gets angy abut this but there's not much we can do about it
                #[allow(mutable_borrow_reservation_conflict)]
                cert.append_extension(
                    extension::SubjectAlternativeName::new()
                        .critical()
                        .uri(&hostname)
                        .build(&ctx)
                        .unwrap(),
                )
                .unwrap();
            }
            cert.set_pubkey(&privkey).unwrap();
            cert.sign(&privkey, MessageDigest::sha512()).unwrap();
            let cert = cert.build();
            let privkey = rustls::PrivateKey(privkey.private_key_to_der().unwrap());
            Some(Arc::new(rustls::sign::CertifiedKey {
                cert: vec![
                    rustls::Certificate(cert.to_der().unwrap()),
                    rustls::Certificate(pubkey.to_der().unwrap()),
                ],
                key: Arc::new(RsaSigningKey::new(&privkey).unwrap()),
                ocsp: None,
                sct_list: None,
            }))
        } else {
            None
        }
    }
}
