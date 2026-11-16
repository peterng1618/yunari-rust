//! One port serves both worlds: a phone's TLS handshake and the loopback
//! admin's plain HTTP. The first byte decides which.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::ConnectInfo;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tower::Service;

use crate::config::HostConfig;
use crate::http::{App, Shared};

/// The byte every TLS record starts with.
const TLS_HANDSHAKE_BYTE: u8 = 0x16;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SOCKET_CONNECTIONS: usize = 256;

#[derive(Debug)]
pub struct HostError(pub String);

impl std::fmt::Display for HostError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for HostError {}

impl From<std::io::Error> for HostError {
    fn from(error: std::io::Error) -> Self {
        Self(format!("the host could not bind: {error}"))
    }
}

pub struct Server {
    pub addr: SocketAddr,
    /// `http://127.0.0.1:<port>`: what the loopback admin opens.
    pub local_url: String,
    /// `https://<name>:<port>` when phones can go secure.
    pub secure_url: String,
    pub fingerprint: Option<String>,
    /// The root certificate as PEM, when the host serves TLS.
    pub certificate_pem: Option<String>,
    shutdown: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Server {
    /// Stop accepting, drop live connections and wait for the loop to finish.
    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).ok();
        }
        self.handle.await.ok();
    }
}

/// The app for a settings directory: reads its files and mints its TLS root.
pub fn app(config: HostConfig) -> Shared {
    let settings = config.settings_dir();
    std::fs::create_dir_all(&settings).ok();
    let authority = crate::tls::Authority::load(&settings).ok();
    Arc::new(App {
        identity: crate::identity::HostIdentity::load_or_mint(&settings),
        displays: std::sync::Mutex::new(crate::displays::DisplayStore::load(&settings)),
        pairing: std::sync::Mutex::new(crate::pairing::Pairing::default()),
        library: yunari_library::Library::new(config.themes_dir()),
        bound_port: AtomicU16::new(config.port),
        streams: crate::stream::Streams::new(),
        registry: providers(),
        authority,
        config,
    })
}

/// The providers this PC can read through. A host with none still serves the
/// clock and says why every other key is a gap.
fn providers() -> yunari_providers::registry::Registry {
    let mut registry = yunari_providers::registry::Registry::new();
    registry.add(yunari_providers::clock::ClockProvider::new());
    registry.add(yunari_providers::system::SystemProvider::new());
    registry
}

pub async fn start(app: Shared, lan: bool) -> Result<Server, HostError> {
    let host = if lan {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    let mut listener = None;
    for port in app.config.ports() {
        match TcpListener::bind(SocketAddr::new(host, port)).await {
            Ok(bound) => {
                listener = Some(bound);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let listener = listener.ok_or_else(|| {
        HostError(format!(
            "Ports {}–{} are all in use. Free one, or pass a different port.",
            app.config.port,
            app.config.port.saturating_add(crate::config::MAX_PORT_ATTEMPTS - 1)
        ))
    })?;
    let addr = listener.local_addr()?;
    app.bound_port.store(addr.port(), Ordering::Relaxed);

    let tls = app
        .authority
        .as_ref()
        .map(|authority| tokio_rustls::TlsAcceptor::from(authority.server_config()));
    crate::stream::spawn(app.clone());
    let router = crate::http::router(app.clone());
    let live = Arc::new(AtomicUsize::new(0));
    let (shutdown, mut stopped) = oneshot::channel();

    let handle = tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted,
            };
            let (stream, peer) = match accepted {
                Ok(accepted) => accepted,
                Err(_) => continue,
            };
            if live.load(Ordering::Relaxed) >= MAX_SOCKET_CONNECTIONS {
                continue;
            }
            live.fetch_add(1, Ordering::Relaxed);
            let guard = LiveGuard(live.clone());
            let tls = tls.clone();
            let router = router.clone();
            tokio::spawn(async move {
                let _guard = guard;
                if serve(stream, tls, router, peer).await.is_err() {
                    // A connection that ends early is the phone's business.
                }
            });
        }
    });

    let local_url = format!("http://127.0.0.1:{}", addr.port());
    let secure_url = format!("https://localhost:{}", addr.port());
    Ok(Server {
        addr,
        local_url,
        secure_url,
        fingerprint: app.authority.as_ref().map(crate::tls::Authority::fingerprint_hex),
        certificate_pem: app
            .authority
            .as_ref()
            .map(|authority| authority.certificate_pem().to_owned()),
        shutdown: Some(shutdown),
        handle,
    })
}

async fn serve(
    stream: TcpStream,
    tls: Option<tokio_rustls::TlsAcceptor>,
    router: axum::Router,
    peer: SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Peek rather than read: whichever protocol this is must still receive the
    // first byte.
    let mut first = [0u8; 1];
    let read = tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.peek(&mut first)).await;
    let count = match read {
        Ok(Ok(count)) => count,
        _ => return Ok(()),
    };
    if count == 0 {
        return Ok(());
    }
    let router = router.clone();
    let service = tower::service_fn(move |mut request: axum::http::Request<hyper::body::Incoming>| {
        request.extensions_mut().insert(ConnectInfo(peer));
        let mut router = router.clone();
        async move { router.call(request).await }
    });
    let service = TowerToHyperService::new(service);
    let mut builder = Builder::new(TokioExecutor::new());
    builder.http1().keep_alive(true);

    if first[0] == TLS_HANDSHAKE_BYTE
        && let Some(acceptor) = tls
    {
        let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await??;
        builder
            .serve_connection(TokioIo::new(stream), service)
            .await?;
    } else {
        builder
            .serve_connection(TokioIo::new(stream), service)
            .await?;
    }
    Ok(())
}

struct LiveGuard(Arc<AtomicUsize>);

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn config(name: &str) -> HostConfig {
        let app_dir = std::env::temp_dir().join(format!(
            "yunari-server-{name}-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        HostConfig::resolve(Some(app_dir), Some(0), false)
    }

    async fn start_host(name: &str) -> Server {
        let app = app(config(name));
        start(app, false).await.expect("a free port")
    }

    async fn plain(server: &Server, request: &str) -> String {
        let mut stream = TcpStream::connect(server.addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }

    async fn secure(server: &Server, ca_pem: &str, request: &str) -> String {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(rustls::pki_types::CertificateDer::from(pem_der(ca_pem)))
            .unwrap();
        let client = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
        let tcp = TcpStream::connect(server.addr).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let mut stream = connector.connect(name, tcp).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }

    fn pem_der(pem: &str) -> Vec<u8> {
        let body: String = pem
            .lines()
            .skip_while(|line| !line.starts_with("-----BEGIN"))
            .skip(1)
            .take_while(|line| !line.starts_with("-----END"))
            .collect();
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(body).unwrap()
    }

    #[tokio::test]
    async fn health_answers_plain_and_secure_on_one_port() {
        let server = start_host("health").await;
        let plain = plain(
            &server,
            "GET /api/health HTTP/1.1
Host: 127.0.0.1
Connection: close

",
        )
        .await;
        assert!(plain.starts_with("HTTP/1.1 200"), "{plain}");
        assert!(plain.contains("\"ok\":true"), "{plain}");
        let ca_pem = server.certificate_pem.clone().expect("a TLS root");
        let secure = secure(
            &server,
            &ca_pem,
            "GET /api/health HTTP/1.1
Host: localhost
Connection: close

",
        )
        .await;
        assert!(secure.starts_with("HTTP/1.1 200"), "{secure}");
        assert!(secure.contains("\"ok\":true"), "{secure}");
        server.stop().await;
    }

    #[tokio::test]
    async fn a_foreign_origin_is_refused_on_the_loopback_admin() {
        let server = start_host("origin").await;
        let response = plain(
            &server,
            "GET /api/displays HTTP/1.1
Host: localhost:5227
Origin: http://evil.example
Connection: close

",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
        assert!(response.contains("another origin"), "{response}");
        let same = plain(
            &server,
            "GET /api/displays HTTP/1.1
Host: localhost:5227
Origin: http://localhost:5227
Connection: close

",
        )
        .await;
        assert!(same.starts_with("HTTP/1.1 200"), "{same}");
        server.stop().await;
    }

    #[tokio::test]
    async fn the_identity_is_served_without_a_token_to_an_unpaired_reader() {
        let server = start_host("host").await;
        let response = plain(
            &server,
            "GET /api/host HTTP/1.1
Host: localhost
Connection: close

",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"name\""));
        assert!(!response.contains("\"token\""));
        server.stop().await;
    }
}
