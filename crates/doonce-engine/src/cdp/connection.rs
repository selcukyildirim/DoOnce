//! Minimal Chrome DevTools Protocol client over a single WebSocket using
//! flattened target sessions.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::driver::DriverError;

#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>>;

#[derive(Debug, Clone)]
pub struct CdpError {
    pub code: i64,
    pub message: String,
}

impl From<CdpError> for DriverError {
    fn from(e: CdpError) -> Self {
        let m = e.message.to_ascii_lowercase();
        if m.contains("context") || m.contains("navigat") || m.contains("target closed") || m.contains("detached") {
            DriverError::Transient(e.message)
        } else {
            DriverError::Browser(e.message)
        }
    }
}

#[derive(Clone)]
pub struct Connection {
    outgoing: mpsc::UnboundedSender<Message>,
    pending: Pending,
    events: broadcast::Sender<CdpEvent>,
    next_id: Arc<AtomicU64>,
}

impl Connection {
    pub async fn connect(ws_url: &str) -> Result<Self, DriverError> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| DriverError::Browser(format!("could not connect to Chrome DevTools: {e}")))?;
        let (mut sink, mut stream) = ws.split();
        let (outgoing, mut rx) = mpsc::unbounded_channel::<Message>();
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(1024);

        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
        });

        let reader_pending = pending.clone();
        let reader_events = events.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let Message::Text(text) = msg else { continue };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    if let Some(tx) = reader_pending.lock().await.remove(&id) {
                        let result = match value.get("error") {
                            Some(err) => Err(CdpError {
                                code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
                                message: err.get("message").and_then(Value::as_str).unwrap_or("error").to_string(),
                            }),
                            None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = tx.send(result);
                    }
                } else if let Some(method) = value.get("method").and_then(Value::as_str) {
                    let _ = reader_events.send(CdpEvent {
                        method: method.to_string(),
                        params: value.get("params").cloned().unwrap_or(Value::Null),
                        session_id: value.get("sessionId").and_then(Value::as_str).map(str::to_string),
                    });
                }
            }
            // Socket closed: fail everything still waiting.
            for (_, tx) in reader_pending.lock().await.drain() {
                let _ = tx.send(Err(CdpError { code: -1, message: "browser disconnected".into() }));
            }
        });

        Ok(Connection { outgoing, pending, events, next_id: Arc::new(AtomicU64::new(1)) })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    pub async fn send(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value, DriverError> {
        self.send_timeout(session, method, params, Duration::from_secs(60)).await
    }

    pub async fn send_timeout(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, DriverError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = Value::String(s.to_string());
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        if self.outgoing.send(Message::Text(msg.to_string().into())).is_err() {
            self.pending.lock().await.remove(&id);
            return Err(DriverError::Disconnected);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) if e.code == -1 => Err(DriverError::Disconnected),
            Ok(Ok(Err(e))) => Err(e.into()),
            Ok(Err(_)) => Err(DriverError::Disconnected),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(DriverError::Timeout(format!("{method} did not answer in {}s", timeout.as_secs())))
            }
        }
    }
}
