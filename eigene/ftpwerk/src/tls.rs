//! FTPS: TLS-Verbindung mit eigener Zertifikatsprüfung.
//!
//! Zertifikate von bekannten Stellen (Mozilla-Liste) werden wie üblich geprüft. Bei allen anderen
//! – meist selbst signierte Zertifikate auf eigenen Servern – meldet die Prüfung den
//! SHA-256-Fingerabdruck zurück; der Benutzer entscheidet dann, ob er ihm dauerhaft vertraut.

use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, SignatureScheme, StreamOwned};
use sha2::{Digest, Sha256};

use crate::model::{RErr, Res};
use crate::sprache::tf;

pub type TlsStream = StreamOwned<ClientConnection, TcpStream>;

/// Fingerabdruck als «AB:CD:…» (SHA-256 über das DER-Zertifikat).
pub fn fingerprint(der: &[u8]) -> String {
    let h = Sha256::digest(der);
    h.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

type Pending = Arc<Mutex<Option<(String, bool, String)>>>;

#[derive(Debug)]
struct Verifier {
    inner: Arc<WebPkiServerVerifier>,
    algs: WebPkiSupportedAlgorithms,
    /// Fingerabdrücke, denen der Benutzer für diesen Server schon vertraut.
    trusted: Vec<String>,
    pending: Pending,
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity.as_ref());
        if self.trusted.iter().any(|t| *t == fp) {
            return Ok(ServerCertVerified::assertion());
        }
        match self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Ok(mut p) = self.pending.lock() {
                    *p = Some((fp, !self.trusted.is_empty(), e.to_string()));
                }
                Err(e)
            }
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// Alles, was Steuer- und Datenverbindung gemeinsam brauchen. Beide teilen sich dieselbe
/// Konfiguration, damit der Server die TLS-Sitzung der Steuerverbindung wiedererkennt
/// (viele Server verlangen das für die Datenverbindung).
#[derive(Clone)]
pub struct TlsCtx {
    cfg: Arc<ClientConfig>,
    name: ServerName<'static>,
    pending: Pending,
}

impl TlsCtx {
    pub fn new(host: &str, trusted: &[String]) -> Res<TlsCtx> {
        let fail = |e: &dyn std::fmt::Display| RErr::Msg(tf("TLS konnte nicht vorbereitet werden: {e}", &[("e", e)]));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone()).build().map_err(|e| fail(&e))?;
        let pending: Pending = Arc::new(Mutex::new(None));
        let verifier = Verifier {
            inner,
            algs: provider.signature_verification_algorithms,
            trusted: trusted.to_vec(),
            pending: pending.clone(),
        };
        let cfg = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| fail(&e))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        let name = ServerName::try_from(host.trim().to_string()).map_err(|_| RErr::Msg(tf("Ungültiger Servername: {host}", &[("host", &host)])))?;
        Ok(TlsCtx { cfg: Arc::new(cfg), name, pending })
    }

    pub fn wrap(&self, tcp: TcpStream) -> Res<TlsStream> {
        let conn = ClientConnection::new(self.cfg.clone(), self.name.clone()).map_err(|e| RErr::Msg(tf("Sichere Verbindung fehlgeschlagen: {e}", &[("e", &e)])))?;
        Ok(StreamOwned::new(conn, tcp))
    }

    /// Hat die Prüfung ein Zertifikat abgelehnt, steht hier der Fingerabdruck zum Nachfragen.
    pub fn untrusted(&self) -> Option<RErr> {
        let p = self.pending.lock().ok()?.clone()?;
        Some(RErr::UntrustedCert { fp: p.0, changed: p.1, detail: p.2 })
    }

    pub fn clear_untrusted(&self) {
        if let Ok(mut p) = self.pending.lock() {
            *p = None;
        }
    }
}
