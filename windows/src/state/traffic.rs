use crate::logic::health::now_ms;
use crate::logic::kernel::KernelKind;
use crate::logic::traffic::{TrafficLedger, TrafficSample};
use crate::services::core_client::CoreClient;
use crate::services::paths;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy)]
pub struct TrafficPoint {
    pub time: u64,
    pub upload: u64,
    pub download: u64,
}

pub struct TrafficData {
    pub upload_rate: u64,
    pub download_rate: u64,
    pub session_upload: u64,
    pub session_download: u64,
    pub lifetime_upload: u64,
    pub lifetime_download: u64,
    pub connected: bool,
    pub points: VecDeque<TrafficPoint>,
    ledger: TrafficLedger,
    last_save: u64,
}

impl TrafficData {
    pub fn load() -> Self {
        let ledger = std::fs::read(paths::data_root().join("traffic-history.json"))
            .ok()
            .and_then(|data| serde_json::from_slice::<TrafficLedger>(&data).ok())
            .unwrap_or_default();
        Self {
            upload_rate: 0,
            download_rate: 0,
            session_upload: 0,
            session_download: 0,
            lifetime_upload: ledger.lifetime_upload,
            lifetime_download: ledger.lifetime_download,
            connected: false,
            points: VecDeque::new(),
            ledger,
            last_save: 0,
        }
    }

    fn save(&mut self) {
        if let Ok(data) = serde_json::to_vec(&self.ledger) {
            let _ = paths::write_private(&paths::data_root().join("traffic-history.json"), &data);
            self.last_save = crate::logic::health::now_ms();
        }
    }

    fn ingest(&mut self, sample: TrafficSample, run_id: &str) {
        self.connected = true;
        self.upload_rate = sample.up;
        self.download_rate = sample.down;
        self.points.push_back(TrafficPoint {
            time: crate::logic::health::now_ms(),
            upload: sample.up,
            download: sample.down,
        });
        while self.points.len() > 60 {
            self.points.pop_front();
        }
        if let (Some(up), Some(down)) = (sample.up_total, sample.down_total) {
            self.session_upload = up;
            self.session_download = down;
            self.ledger.record(up, down, run_id);
            self.lifetime_upload = self.ledger.lifetime_upload;
            self.lifetime_download = self.ledger.lifetime_download;
            if crate::logic::health::now_ms().saturating_sub(self.last_save) >= 5000 {
                self.save();
            }
        }
    }
}

pub fn start(
    client: CoreClient,
    data: Arc<Mutex<TrafficData>>,
    stop: Arc<AtomicBool>,
    run_id: String,
) {
    {
        let mut guard = data.lock().unwrap();
        guard.session_upload = 0;
        guard.session_download = 0;
        guard.points.clear();
        guard.connected = false;
    }
    if client.kind == KernelKind::Xray {
        crate::services::workers::spawn(move || poll_xray(client, data, stop, run_id));
        return;
    }
    crate::services::workers::spawn(move || {
        while !stop.load(Ordering::SeqCst) && crate::services::workers::valid() {
            let Ok(mut child) = client.traffic_stream() else {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            };
            let Some(stdout) = child.stdout.take() else {
                let _ = child.kill();
                continue;
            };
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if stop.load(Ordering::SeqCst) {
                    let _ = child.kill();
                    break;
                }
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(sample) = serde_json::from_str::<TrafficSample>(&line) {
                    if let Ok(mut guard) = data.lock() {
                        guard.ingest(sample, &run_id);
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            if let Ok(mut guard) = data.lock() {
                guard.connected = false;
                guard.upload_rate = 0;
                guard.download_rate = 0;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if let Ok(mut guard) = data.lock() {
            guard.save();
            guard.connected = false;
            guard.upload_rate = 0;
            guard.download_rate = 0;
        }
    });
}

/// Xray 没有流量事件流：每秒查询一次 StatsService，按累计值差分得到速率。
fn poll_xray(
    client: CoreClient,
    data: Arc<Mutex<TrafficData>>,
    stop: Arc<AtomicBool>,
    run_id: String,
) {
    let mut previous: Option<(u64, u64, u64)> = None;
    while !stop.load(Ordering::SeqCst) && crate::services::workers::valid() {
        match client.traffic_totals() {
            Ok((up, down)) => {
                let now = now_ms();
                let (up_rate, down_rate) = match previous {
                    Some((last_time, last_up, last_down)) if now > last_time => {
                        let seconds = (now - last_time) as f64 / 1000.0;
                        let up_delta = up.saturating_sub(last_up) as f64;
                        let down_delta = down.saturating_sub(last_down) as f64;
                        ((up_delta / seconds) as u64, (down_delta / seconds) as u64)
                    }
                    _ => (0, 0),
                };
                previous = Some((now, up, down));
                if let Ok(mut guard) = data.lock() {
                    guard.ingest(
                        TrafficSample {
                            up: up_rate,
                            down: down_rate,
                            up_total: Some(up),
                            down_total: Some(down),
                        },
                        &run_id,
                    );
                }
            }
            Err(_) => {
                if let Ok(mut guard) = data.lock() {
                    guard.connected = false;
                    guard.upload_rate = 0;
                    guard.download_rate = 0;
                }
                previous = None;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    if let Ok(mut guard) = data.lock() {
        guard.save();
        guard.connected = false;
        guard.upload_rate = 0;
        guard.download_rate = 0;
    }
}
