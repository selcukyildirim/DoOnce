//! Minimal CDP JSON-RPC client over a WebSocket.
//!
//! One writer task owns the socket sink, one reader task dispatches responses
//! to waiting callers and events to subscribers. The shared state is small and
//! lock scopes are short; no lock is held across an await point.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::error::{BrowserError, Result};

#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, BrowserError>>>>>;

#[derive(Clone)]
pub struct Connection {
    tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    events: broadcast::Sender<CdpEvent>,
    next_id: Arc<AtomicU64>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").finish_non_exhaustive()
    }
}

impl Connection {
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
        let (mut sink, mut stream) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();

        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        });

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(512);
        let reader_pending = pending.clone();
        let reader_events = events.clone();

        tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                let Ok(message) = message else { break };
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                    Message::Close(_) => break,
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };

                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    let sender = reader_pending.lock().expect("pending lock").remove(&id);
                    if let Some(sender) = sender {
                        let result = match value.get("error") {
                            Some(error) => Err(BrowserError::Browser(
                                error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("CDP command failed")
                                    .to_string(),
                            )),
                            None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = sender.send(result);
                    }
                    continue;
                }

                if let Some(method) = value.get("method").and_then(Value::as_str) {
                    let _ = reader_events.send(CdpEvent {
                        method: method.to_string(),
                        params: value.get("params").cloned().unwrap_or(Value::Null),
                        session_id: value
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
                }
            }

            let mut pending = reader_pending.lock().expect("pending lock");
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(BrowserError::Disconnected));
            }
        });

        Ok(Connection {
            tx,
            pending,
            events,
            next_id: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    pub async fn send(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        self.send_timeout(session_id, method, params, Duration::from_secs(30))
            .await
    }

    pub async fn send_timeout(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending lock")
            .insert(id, sender);

        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session_id {
            message["sessionId"] = json!(session);
        }
        let text =
            serde_json::to_string(&message).map_err(|e| BrowserError::Protocol(e.to_string()))?;
        if self.tx.send(text).is_err() {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err(BrowserError::Disconnected);
        }

        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(BrowserError::Disconnected),
            Err(_) => {
                self.pending.lock().expect("pending lock").remove(&id);
                Err(BrowserError::Timeout(format!(
                    "{method} did not answer within {}s",
                    timeout.as_secs()
                )))
            }
        }
    }
}
