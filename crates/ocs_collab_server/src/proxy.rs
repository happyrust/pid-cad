//! Local TCP fault-injection proxy for network-only disconnect, latency, and
//! frame-drop simulation without restarting or touching the server process (#22).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// Controls a running local TCP fault-injection proxy.
pub struct TcpFaultProxy {
    local_addr: SocketAddr,
    accepting: Arc<AtomicBool>,
    latency_ms: Arc<AtomicU64>,
    drop_next_chunks: Arc<AtomicUsize>,
    clients: Arc<Mutex<HashMap<u64, oneshot::Sender<()>>>>,
    task: tokio::task::JoinHandle<()>,
}

impl TcpFaultProxy {
    /// Starts a TCP proxy bound to `127.0.0.1:0` forwarding to `upstream`.
    pub async fn start(upstream: SocketAddr) -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
        let local_addr = listener.local_addr()?;
        let accepting = Arc::new(AtomicBool::new(true));
        let latency_ms = Arc::new(AtomicU64::new(0));
        let drop_next_chunks = Arc::new(AtomicUsize::new(0));
        let clients: Arc<Mutex<HashMap<u64, oneshot::Sender<()>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let acc_clone = accepting.clone();
        let lat_clone = latency_ms.clone();
        let drop_clone = drop_next_chunks.clone();
        let clients_clone = clients.clone();
        let next_id = Arc::new(AtomicU64::new(1));

        let task = tokio::spawn(async move {
            while let Ok((client_stream, _)) = listener.accept().await {
                if !acc_clone.load(Ordering::SeqCst) {
                    drop(client_stream);
                    continue;
                }
                let Ok(server_stream) = TcpStream::connect(upstream).await else {
                    drop(client_stream);
                    continue;
                };
                let id = next_id.fetch_add(1, Ordering::SeqCst);
                let (kill_tx, kill_rx) = oneshot::channel();
                clients_clone
                    .lock()
                    .expect("proxy clients mutex")
                    .insert(id, kill_tx);
                let lat = lat_clone.clone();
                let drops = drop_clone.clone();
                let map = clients_clone.clone();
                tokio::spawn(async move {
                    relay(client_stream, server_stream, lat, drops, kill_rx).await;
                    map.lock().expect("proxy clients mutex").remove(&id);
                });
            }
        });

        Ok(Self {
            local_addr,
            accepting,
            latency_ms,
            drop_next_chunks,
            clients,
            task,
        })
    }

    /// Address clients should connect to instead of the server directly.
    pub fn addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Abruptly severs a specific proxied TCP connection without sending a WS
    /// close frame (network-only disconnect).
    pub fn cut_client(&self, id: u64) -> bool {
        if let Some(tx) = self.clients.lock().expect("proxy clients mutex").remove(&id) {
            let _ = tx.send(());
            true
        } else {
            false
        }
    }

    /// Abruptly severs all currently active proxied TCP connections.
    pub fn cut_all(&self) -> usize {
        let mut map = self.clients.lock().expect("proxy clients mutex");
        let count = map.len();
        for (_, tx) in map.drain() {
            let _ = tx.send(());
        }
        count
    }

    /// Enables or blocks new incoming client connections.
    pub fn set_accepting(&self, accepting: bool) {
        self.accepting.store(accepting, Ordering::SeqCst);
    }

    /// Injects artificial forwarding latency on each forwarded chunk.
    pub fn set_latency(&self, delay: Duration) {
        self.latency_ms
            .store(delay.as_millis() as u64, Ordering::SeqCst);
    }

    /// Causes the next `count` client->server TCP chunks to be silently dropped.
    pub fn drop_next_chunks(&self, count: usize) {
        self.drop_next_chunks.store(count, Ordering::SeqCst);
    }

    /// Number of currently active proxied client connections.
    pub fn active_connections(&self) -> usize {
        self.clients.lock().expect("proxy clients mutex").len()
    }
}

impl Drop for TcpFaultProxy {
    fn drop(&mut self) {
        self.cut_all();
        self.task.abort();
    }
}

async fn relay(
    client: TcpStream,
    server: TcpStream,
    latency_ms: Arc<AtomicU64>,
    drop_next_chunks: Arc<AtomicUsize>,
    mut kill_rx: oneshot::Receiver<()>,
) {
    let (mut c_read, mut c_write) = client.into_split();
    let (mut s_read, mut s_write) = server.into_split();

    let lat_c2s = latency_ms.clone();
    let c2s = async move {
        let mut buf = [0u8; 4096];
        loop {
            match c_read.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let delay = lat_c2s.load(Ordering::SeqCst);
                    if delay > 0 {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                    let should_drop = drop_next_chunks
                        .try_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                            if v > 0 { Some(v - 1) } else { None }
                        })
                        .is_ok();
                    if should_drop {
                        continue;
                    }
                    if s_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
    };

    let s2c = async move {
        let mut buf = [0u8; 4096];
        loop {
            match s_read.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let delay = latency_ms.load(Ordering::SeqCst);
                    if delay > 0 {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                    if c_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
    };

    tokio::select! {
        _ = c2s => {}
        _ = s2c => {}
        _ = &mut kill_rx => {}
    }
}
