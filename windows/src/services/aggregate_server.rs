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
    stop: Arc<std::sync::atomic::AtomicBool>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl AggregateServer {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_string(),
            payload: Arc::new(Mutex::new(Vec::new())),
            started: Arc::new(Mutex::new(false)),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            worker: Mutex::new(None),
        }
    }

    pub fn address(&self) -> String {
        format!(
            "http://127.0.0.1:{AGGREGATE_PORT}/{}/config.yaml",
            self.token
        )
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
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        *self.started.lock().unwrap() = true;
        self.stop.store(false, std::sync::atomic::Ordering::SeqCst);
        let payload = Arc::clone(&self.payload);
        let token = self.token.clone();
        let stop = Arc::clone(&self.stop);
        *self.worker.lock().unwrap() = Some(std::thread::spawn(move || {
            let mut handlers = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if handlers.len() >= 16 {
                            drop(stream);
                        } else {
                            let payload = Arc::clone(&payload);
                            let token = token.clone();
                            handlers
                                .push(std::thread::spawn(move || handle(stream, &token, &payload)));
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(30))
                    }
                    Err(_) => break,
                }
                let mut index = 0;
                while index < handlers.len() {
                    if handlers[index].is_finished() {
                        let _ = handlers.swap_remove(index).join();
                    } else {
                        index += 1;
                    }
                }
            }
            for handler in handlers {
                let _ = handler.join();
            }
        }));
        Ok(())
    }
    pub fn stop(&self) -> Result<(), String> {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            worker.join().map_err(|_| "聚合订阅服务退出失败")?;
        }
        *self.started.lock().unwrap() = false;
        Ok(())
    }
}

fn handle(mut stream: TcpStream, token: &str, payload: &Arc<Mutex<Vec<u8>>>) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return;
        };
        let _ = stream.set_read_timeout(Some(remaining));
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
