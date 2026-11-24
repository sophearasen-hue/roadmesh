//! Accept loops: TLS handshake (with a deadline), device identity from the client
//! certificate, then the broker session.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;

use crate::broker::{Broker, Peer};
use crate::ca::{Attestation, CaKey, device_id};
use crate::devca::device_key_of;
use crate::monitor::Metric;

/// TLS handshakes in progress at most (slow-handshake floods).
pub const MAX_HANDSHAKES: usize = 2048;
/// A handshake must finish within this time.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Which port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Port {
    /// MQTT with mutual TLS.
    Mqtt,
    /// Enrolment (no client certificate).
    Enrol,
}

/// Accept connections forever.
pub async fn accept_loop<K, D, A>(
    broker: Arc<Broker<K, D, A>>,
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    port: Port,
) where
    K: CaKey + Send + 'static,
    D: CaKey + Send + 'static,
    A: Attestation + Send + 'static,
{
    let acceptor = TlsAcceptor::from(tls);
    let handshakes = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    loop {
        let Ok((tcp, addr)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let Ok(permit) = handshakes.clone().try_acquire_owned() else {
            broker.monitor.inc(Metric::SessionsRefused);
            continue; // dropped: too many handshakes in flight
        };
        let (b, acceptor) = (broker.clone(), acceptor.clone());
        tokio::spawn(async move {
            let _ = tcp.set_nodelay(true);
            let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await;
            drop(permit);
            let Ok(Ok(stream)) = tls else {
                b.monitor.inc(Metric::SessionsRefused);
                return;
            };
            let Some(peer) = identify(&stream, addr, port) else {
                b.monitor.inc(Metric::SessionsRefused);
                return;
            };
            b.serve(stream, peer).await;
        });
    }
}

fn identify<S>(
    stream: &tokio_rustls::server::TlsStream<S>,
    addr: SocketAddr,
    port: Port,
) -> Option<Peer> {
    match port {
        Port::Enrol => Some(Peer::Enrol(addr.ip())),
        Port::Mqtt => {
            // rustls has already verified the chain to the device CA
            let certs = stream.get_ref().1.peer_certificates()?;
            let key = device_key_of(certs.first()?)?;
            Some(Peer::Device(device_id(&key)))
        }
    }
}
