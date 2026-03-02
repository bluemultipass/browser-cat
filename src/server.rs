/// Streaming HTTP server.
///
/// Binds on 127.0.0.1:0 (OS-assigned port), serves a single GET / that
/// streams HTML chunks as they arrive. In persist mode the server stays
/// alive between requests and re-serves buffered content.
use axum::{
    Router,
    body::Body,
    http::{Response, StatusCode, header},
    routing::get,
};
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::{Notify, broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

// ── Server config ─────────────────────────────────────────────────────────────

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub persist: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 0,
            persist: false,
        }
    }
}

// ── Shared state ──────────────────────────────────────────────────────────────

/// Shared between the server handler and the main thread that feeds chunks.
struct State {
    /// Replay buffer for persist mode — stores all chunks seen so far.
    buffer: Mutex<Vec<Bytes>>,
    /// Broadcast channel for live chunks; all active handlers receive them.
    live_tx: broadcast::Sender<Bytes>,
    /// Signals that the input stream has ended.
    done: Notify,
    persist: bool,
    /// Fired by the handler after it finishes streaming a response (non-persist).
    served: Arc<Notify>,
    /// Signals axum to stop accepting new connections.
    shutdown: Arc<Notify>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Handle returned from [`serve`]. Use it to feed chunks and signal completion.
pub struct ServerHandle {
    state: Arc<State>,
    /// Bound address (use `.port()` to get the OS-assigned port).
    pub addr: SocketAddr,
}

impl ServerHandle {
    /// Send a chunk to connected browsers.
    pub fn send(&self, chunk: Bytes) {
        let mut buf = self.state.buffer.lock().unwrap();
        buf.push(chunk.clone());
        drop(buf);
        let _ = self.state.live_tx.send(chunk);
    }

    /// Signal that all input has been read.
    ///
    /// In non-persist mode the server shuts down after the handler finishes
    /// streaming the response. Call [`wait_served`] to wait for that before
    /// exiting.
    pub fn finish(&self) {
        // notify_one stores a permit so the handler sees it even if it
        // subscribes after this call.
        self.state.done.notify_one();
        // Shutdown is triggered by the handler itself after streaming completes,
        // so we do NOT fire self.shutdown here.
    }

    /// Wait until a response has been fully streamed to the browser.
    /// Returns immediately in persist mode (server stays alive until Ctrl-C).
    pub async fn wait_served(&self) {
        if !self.state.persist {
            self.state.served.notified().await;
        }
    }
}

/// Bind the server, return a handle for feeding chunks and the bound address.
/// Calls `on_bind` with the bound address synchronously before returning so
/// the caller can open a browser URL pointing at the right port.
pub async fn serve<F>(cfg: ServerConfig, on_bind: F) -> ServerHandle
where
    F: FnOnce(SocketAddr),
{
    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port)
        .parse()
        .expect("invalid bind address");

    let listener = TcpListener::bind(addr)
        .await
        .expect("failed to bind server");

    let bound = listener.local_addr().expect("no local addr");
    on_bind(bound);

    let (live_tx, _) = broadcast::channel::<Bytes>(256);
    let shutdown = Arc::new(Notify::new());
    let served = Arc::new(Notify::new());

    let state = Arc::new(State {
        buffer: Mutex::new(Vec::new()),
        live_tx,
        done: Notify::new(),
        persist: cfg.persist,
        served: Arc::clone(&served),
        shutdown: Arc::clone(&shutdown),
    });

    let state_clone = Arc::clone(&state);
    let shutdown_clone = Arc::clone(&shutdown);

    // Spawn the axum server task.
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(handle_root))
            .with_state(Arc::clone(&state_clone));

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_clone.notified().await;
            })
            .await
            .unwrap();
    });

    ServerHandle { state, addr: bound }
}

// ── Request handler ───────────────────────────────────────────────────────────

async fn handle_root(
    axum::extract::State(state): axum::extract::State<Arc<State>>,
) -> Response<Body> {
    // Replay already-buffered chunks, then subscribe to live ones.
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::convert::Infallible>>(256);

    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        // Send buffered content first.
        let buffered: Vec<Bytes> = state_clone.buffer.lock().unwrap().clone();
        for chunk in buffered {
            if tx.send(Ok(chunk)).await.is_err() {
                return;
            }
        }

        // Then subscribe to live chunks.
        let mut live_rx = state_clone.live_tx.subscribe();
        loop {
            tokio::select! {
                chunk = live_rx.recv() => {
                    match chunk {
                        Ok(bytes) => {
                            if tx.send(Ok(bytes)).await.is_err() {
                                return;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    }
                }
                _ = state_clone.done.notified() => {
                    // Drain any remaining live chunks, then stop.
                    while let Ok(bytes) = live_rx.try_recv() {
                        let _ = tx.send(Ok(bytes)).await;
                    }
                    break;
                }
            }
        }

        // All data has been written into the response body channel.
        // Signal served (wakes main) and shut down the server.
        state_clone.served.notify_one();
        if !state_clone.persist {
            state_clone.shutdown.notify_one();
        }
    });

    let stream = ReceiverStream::new(rx);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::TRANSFER_ENCODING, "chunked")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(body)
        .unwrap()
}
