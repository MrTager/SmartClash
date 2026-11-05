//! 实时连接与 IP 定位：与 macOS 版 NetworkActivityStore 对齐。
//! 轮询内核 /connections（1s），并按优先级逐个解析节点服务器与请求目标的位置（1.2s）。

use crate::logic::model::{CoreConnection, GeoLocation};
use crate::services::core_client::CoreClient;
use crate::services::geoip;
use crate::state::Shared;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ATTEMPT_BACKOFF_MS: u64 = 300_000;

#[derive(Default)]
pub struct ActivityData {
    pub connections: Vec<CoreConnection>,
    pub locations: HashMap<String, GeoLocation>,
    pub connected: bool,
    pub enabled: bool,
    /// 当前内核是否提供实时连接数据。
    pub supported: bool,
    pub resolving: bool,
    attempted: HashMap<String, u64>,
}

pub type SharedActivity = Arc<Mutex<ActivityData>>;

pub fn start(shared: &Shared, client: CoreClient, data: SharedActivity, stop: Arc<AtomicBool>) {
    let supported = client.kind.capabilities().connections;
    {
        let mut guard = data.lock().unwrap();
        guard.connections.clear();
        guard.connected = false;
        guard.enabled = supported;
        guard.supported = supported;
        guard.resolving = false;
        guard.attempted.clear();
    }
    if !supported {
        // Xray 没有连接列表接口；不启动轮询，界面显示能力提示。
        return;
    }
    let poll_data = Arc::clone(&data);
    let poll_client = client.clone();
    let poll_stop = Arc::clone(&stop);
    std::thread::spawn(move || {
        while !poll_stop.load(Ordering::SeqCst) {
            match poll_client.connections() {
                Ok(mut snapshot) => {
                    // 与 macOS 一致：按总流量降序，平局按 id。
                    snapshot
                        .connections
                        .sort_by(|a, b| {
                            let left = a.download.saturating_add(a.upload);
                            let right = b.download.saturating_add(b.upload);
                            right.cmp(&left).then_with(|| a.id.cmp(&b.id))
                        });
                    if let Ok(mut guard) = poll_data.lock() {
                        guard.connections = snapshot.connections;
                        guard.connected = true;
                    }
                }
                Err(_) => {
                    if let Ok(mut guard) = poll_data.lock() {
                        guard.connected = false;
                        guard.connections.clear();
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });

    let geo_shared = Arc::clone(shared);
    let geo_data = Arc::clone(&data);
    let geo_stop = Arc::clone(&stop);
    std::thread::spawn(move || {
        while !geo_stop.load(Ordering::SeqCst) {
            let candidates = collect_candidates(&geo_shared, &geo_data);
            let now = crate::logic::health::now_ms();
            let target = {
                let Ok(guard) = geo_data.lock() else {
                    std::thread::sleep(Duration::from_millis(1200));
                    continue;
                };
                candidates.into_iter().find(|host| {
                    !guard.locations.contains_key(host)
                        && now.saturating_sub(guard.attempted.get(host).copied().unwrap_or(0))
                            > ATTEMPT_BACKOFF_MS
                })
            };
            if let Some(host) = target {
                if let Ok(mut guard) = geo_data.lock() {
                    guard.attempted.insert(host.clone(), now);
                    guard.resolving = true;
                }
                if let Some(location) = geoip::location(&host) {
                    if let Ok(mut guard) = geo_data.lock() {
                        guard.locations.insert(host, location);
                    }
                }
                if let Ok(mut guard) = geo_data.lock() {
                    guard.resolving = false;
                }
            }
            std::thread::sleep(Duration::from_millis(1200));
        }
        geoip::flush();
    });
}

/// 解析优先级与 macOS 一致：当前链路服务器 → 活跃请求的路径服务器与目标 → 其余节点。
fn collect_candidates(shared: &Shared, data: &SharedActivity) -> Vec<String> {
    let (nodes, trace) = crate::state::with_store(shared, |store| {
        (
            store.node_list_nodes(),
            store.active_trace.clone(),
        )
    });
    let index: HashMap<String, &crate::logic::model::ProxyNode> = nodes
        .iter()
        .map(|node| (node.name.clone(), node))
        .collect();

    let mut candidates: Vec<String> = Vec::new();
    let push = |value: Option<String>, candidates: &mut Vec<String>| {
        if let Some(server) = value {
            if !server.is_empty() && !candidates.contains(&server) {
                candidates.push(server);
            }
        }
    };

    // 当前链路（active trace）的服务器最优先。
    for hop in &trace.hops {
        let server = nodes
            .iter()
            .find(|node| node.id == hop.id)
            .and_then(|node| node.payload.get("server"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        push(server, &mut candidates);
    }

    if let Ok(guard) = data.lock() {
        for connection in guard.connections.iter().take(80) {
            for node in connection.route_nodes(&index) {
                push(
                    node.payload
                        .get("server")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    &mut candidates,
                );
            }
            if let Some(key) = connection.metadata.geolocation_key() {
                if !candidates.contains(&key) {
                    candidates.push(key);
                }
            }
        }
    }

    for node in &nodes {
        push(
            node.payload
                .get("server")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            &mut candidates,
        );
    }
    candidates
}

pub fn stop(data: &SharedActivity) {
    if let Ok(mut guard) = data.lock() {
        guard.enabled = false;
        guard.connected = false;
        guard.resolving = false;
        guard.connections.clear();
    }
}
