//! Native TLS transport (`https://` and `wss://`) built directly on `rustls`
//! and `rustls-pki-types` (already in the workspace lockfile, #69).
//!
//! `TlsListener` implements `axum::serve::Listener` with lazy per-connection
//! TLS handshaking inside `TlsStream`'s `AsyncRead`/`AsyncWrite`, so a slow
//! client handshake never stalls `accept()` for other clients.

use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::connect_info::Connected;
use axum::serve::IncomingStream;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};

/// Peer socket address extractor compatible with both plain `TcpListener` and
/// `TlsListener` (`axum::extract::ConnectInfo<PeerAddr>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerAddr(pub SocketAddr);

impl Connected<IncomingStream<'_, TlsListener>> for PeerAddr {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        PeerAddr(*stream.remote_addr())
    }
}

impl Connected<IncomingStream<'_, TcpListener>> for PeerAddr {
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Self {
        PeerAddr(*stream.remote_addr())
    }
}

/// Loads a PEM certificate chain and private key from files into a `rustls`
/// [`ServerConfig`].
pub fn load_server_config(cert_path: &Path, key_path: &Path) -> Result<Arc<ServerConfig>, String> {
    let cert_bytes = std::fs::read(cert_path)
        .map_err(|err| format!("读取 TLS 证书 {} 失败：{err}", cert_path.display()))?;
    let key_bytes = std::fs::read(key_path)
        .map_err(|err| format!("读取 TLS 私钥 {} 失败：{err}", key_path.display()))?;
    load_server_config_pem(&cert_bytes, &key_bytes)
}

/// Parses PEM certificate chain bytes and private key bytes into a `rustls`
/// [`ServerConfig`].
pub fn load_server_config_pem(
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<Arc<ServerConfig>, String> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("解析 TLS 证书 PEM 失败：{err}"))?;
    if certs.is_empty() {
        return Err("TLS 证书文件中没有找到任何 CERTIFICATE 块".to_owned());
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem)
        .map_err(|err| format!("解析 TLS 私钥 PEM 失败：{err}"))?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| format!("初始化 TLS 协议版本失败：{err}"))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|err| format!("加载 TLS 证书/私钥失败：{err}"))?;
    Ok(Arc::new(config))
}

/// An `axum::serve::Listener` that wraps accepted TCP connections in `TlsStream`.
pub struct TlsListener {
    inner: TcpListener,
    config: Arc<ServerConfig>,
}

impl TlsListener {
    pub fn new(inner: TcpListener, config: Arc<ServerConfig>) -> Self {
        Self { inner, config }
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.inner.accept().await {
                Ok((stream, addr)) => match ServerConnection::new(self.config.clone()) {
                    Ok(conn) => return (TlsStream::new(stream, conn), addr),
                    Err(err) => {
                        eprintln!("[ocs_collab_server] 创建 TLS 连接状态失败：{err}");
                    }
                },
                Err(err) => {
                    eprintln!("[ocs_collab_server] 接受 TCP 连接失败：{err}");
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

/// Async server-side TLS stream bridging `tokio::net::TcpStream` and
/// `rustls::ServerConnection`.
pub struct TlsStream {
    io: TcpStream,
    conn: ServerConnection,
    eof: bool,
}

impl TlsStream {
    pub fn new(io: TcpStream, conn: ServerConnection) -> Self {
        Self {
            io,
            conn,
            eof: false,
        }
    }

    fn poll_flush_tls(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.conn.wants_write() {
            let mut adapter = SyncWriteAdapter {
                io: Pin::new(&mut self.io),
                cx,
            };
            match self.conn.write_tls(&mut adapter) {
                Ok(0) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "TLS write_tls returned 0 bytes",
                    )));
                }
                Ok(_) => {}
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Poll::Pending,
                Err(err) => return Poll::Ready(Err(err)),
            }
        }
        Poll::Ready(Ok(()))
    }

    fn poll_read_tls_step(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        let mut adapter = SyncReadAdapter {
            io: Pin::new(&mut self.io),
            cx,
        };
        match self.conn.read_tls(&mut adapter) {
            Ok(0) => {
                self.eof = true;
                Poll::Ready(Ok(0))
            }
            Ok(n) => {
                self.conn
                    .process_new_packets()
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                Poll::Ready(Ok(n))
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

impl AsyncRead for TlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            // 1. Flush any pending TLS handshake/alert records first.
            if this.conn.wants_write() {
                if let Poll::Ready(Err(err)) = this.poll_flush_tls(cx) {
                    return Poll::Ready(Err(err));
                }
            }

            // 2. Deliver already-decrypted plaintext if available.
            let mut reader = this.conn.reader();
            match reader.read(buf.initialize_unfilled()) {
                Ok(0) => {
                    // Clean TLS close_notify EOF.
                    return Poll::Ready(Ok(()));
                }
                Ok(n) => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                Err(err) => return Poll::Ready(Err(err)),
            }

            if this.eof {
                return Poll::Ready(Ok(()));
            }

            // 3. Read more TLS ciphertext from the underlying TcpStream.
            match this.poll_read_tls_step(cx) {
                Poll::Ready(Ok(0)) => {
                    // Peer closed TCP; try draining any final decrypted bytes once more.
                    let mut reader = this.conn.reader();
                    match reader.read(buf.initialize_unfilled()) {
                        Ok(n) => {
                            buf.advance(n);
                            return Poll::Ready(Ok(()));
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                            return Poll::Ready(Ok(()));
                        }
                        Err(err) => return Poll::Ready(Err(err)),
                    }
                }
                Poll::Ready(Ok(_)) => continue,
                Poll::Ready(Err(err)) => {
                    // Flush any TLS alert generated during packet processing.
                    let _ = this.poll_flush_tls(cx);
                    return Poll::Ready(Err(err));
                }
                Poll::Pending => {
                    // If handshake generated outbound frames, ensure they are flushed.
                    if this.conn.wants_write() {
                        if let Poll::Ready(Err(err)) = this.poll_flush_tls(cx) {
                            return Poll::Ready(Err(err));
                        }
                    }
                    return Poll::Pending;
                }
            }
        }
    }
}

impl AsyncWrite for TlsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        loop {
            // Complete handshake before writing application data.
            if this.conn.is_handshaking() {
                if this.conn.wants_write() {
                    match this.poll_flush_tls(cx) {
                        Poll::Ready(Ok(())) => {}
                        Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                if this.conn.wants_read() {
                    match this.poll_read_tls_step(cx) {
                        Poll::Ready(Ok(0)) => {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "peer closed during TLS handshake",
                            )));
                        }
                        Poll::Ready(Ok(_)) => continue,
                        Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
            }

            let written = this.conn.writer().write(buf)?;
            match this.poll_flush_tls(cx) {
                Poll::Ready(Ok(())) | Poll::Pending => return Poll::Ready(Ok(written)),
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.poll_flush_tls(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.io).poll_flush(cx),
            other => other,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.send_close_notify();
        match this.poll_flush_tls(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.io).poll_shutdown(cx),
            other => other,
        }
    }
}

struct SyncReadAdapter<'a, 'b> {
    io: Pin<&'a mut TcpStream>,
    cx: &'a mut Context<'b>,
}

impl Read for SyncReadAdapter<'_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut read_buf = ReadBuf::new(buf);
        match self.io.as_mut().poll_read(self.cx, &mut read_buf) {
            Poll::Ready(Ok(())) => Ok(read_buf.filled().len()),
            Poll::Ready(Err(err)) => Err(err),
            Poll::Pending => Err(io::ErrorKind::WouldBlock.into()),
        }
    }
}

struct SyncWriteAdapter<'a, 'b> {
    io: Pin<&'a mut TcpStream>,
    cx: &'a mut Context<'b>,
}

impl Write for SyncWriteAdapter<'_, '_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.io.as_mut().poll_write(self.cx, buf) {
            Poll::Ready(result) => result,
            Poll::Pending => Err(io::ErrorKind::WouldBlock.into()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.io.as_mut().poll_flush(self.cx) {
            Poll::Ready(result) => result,
            Poll::Pending => Err(io::ErrorKind::WouldBlock.into()),
        }
    }
}
