use crate::logic::configuration;
use crate::logic::model::{ProxyNode, AGGREGATE_PORT};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct AggregateServer {
    token: String,
    payload: Arc<Mutex<Vec<u8>>>,
    started: Arc<Mutex<bool>>,
}

impl AggregateServer {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_string(),
            payload: Arc::new(Mutex::new(Vec::new())),
            started: Arc::new(Mutex::new(false)),
        }
    }

    pub fn address(&self) -> String {
        format!("http://127.0.0.1:{AGGREGATE_PORT}/{}/config.yaml", self.token)
    }

    pub fn is_started(&self) -> bool {
        *self.started.lock().unwrap()
    }

    pub fn update(&self, nodes: &[ProxyNode]) -> Result<(), String> {
        let data = configuration::export(nodes)?;
        *self.payload.lock().unwrap() = data;
        Ok(())
    }

    pub fn start(&self) -> Result<(), String> {
        if self.is_started() {
            return Ok(());
        }
        let listener = TcpListener::bind(("127.0.0.1", AGGREGATE_PORT))
            .map_err(|_| format!("本机订阅端口 {AGGREGATE_PORT} 无法监听，请检查端口冲突"))?;
        *self.started.lock().unwrap() = true;
        let payload = Arc::clone(&self.payload);
        let token = self.token.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let payload = Arc::clone(&payload);
                let token = token.clone();
                std::thread::spawn(move || handle(stream, &token, &payload));
            }
        });
        Ok(())
    }
}

fn handle(mut stream: TcpStream, token: &str, payload: &Arc<Mutex<Vec<u8>>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(n) => {
                request.extend_from_slice(&buffer[..n]);
                if request.len() > 8192 {
                    return;
                }
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let text = String::from_utf8_lossy(&request);
    let first_line = text.lines().next().unwrap_or("");
    let valid = first_line == format!("GET /{token}/config.yaml HTTP/1.1")
        || first_line == format!("GET /{token}/config.yaml HTTP/1.0");
    let body = if valid {
        payload.lock().unwrap().clone()
    } else {
        b"Not found".to_vec()
    };
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/yaml; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        if valid { "200 OK" } else { "404 Not Found" },
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}
