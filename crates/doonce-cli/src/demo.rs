//! A tiny static server for the bundled demo site (`doonce demo`). Bound to
//! localhost only; serves embedded files, nothing from disk.

use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

macro_rules! site {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../../examples/demo-site/", $name)) as &[u8])),*]
    };
}

const FILES: &[(&str, &[u8])] = site![
    "index.html", "contact.html", "contacts.html", "reports.html", "account.html", "help.html",
    "app.js", "style.css",
];

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Binds the demo server. Port 0 picks a free port.
pub async fn bind(port: u16) -> std::io::Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    Ok((listener, addr))
}

pub async fn serve(listener: TcpListener) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else { continue };
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            let Ok(n) = stream.read(&mut buf).await else { return };
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request
                .lines()
                .next()
                .and_then(|l| l.strip_prefix("GET "))
                .and_then(|l| l.split_whitespace().next())
                .unwrap_or("/");
            let path = path.split(['?', '#']).next().unwrap_or("/").trim_start_matches('/');
            let path = if path.is_empty() { "index.html" } else { path };
            let response = match FILES.iter().find(|(name, _)| *name == path) {
                Some((name, body)) => {
                    let mut r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                        content_type(name),
                        body.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(body);
                    r
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot found".to_vec(),
            };
            let _ = stream.write_all(&response).await;
            let _ = stream.shutdown().await;
        });
    }
}
