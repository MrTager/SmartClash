//! 实时连接与 IP 定位：与 macOS 版 NetworkActivityStore 对齐。
//! 轮询内核 /connections（1s），并按优先级逐个解析节点服务器与请求目标的位置（1.2s）。

use crate::logic::chain::{ChainHop, ChainTrace};
use crate::logic::model::{CoreConnection, GeoLocation, ProxyNode};
use crate::services::core_client::CoreClient;
use crate::services::geoip;
use crate::state::Shared;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PREFERRED_BACKOFF_MS: u64 = 10_000;
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
    active_hops: Vec<(ChainHop, Option<String>)>,
    preferred_servers: Vec<String>,
    generation: u64,
}

impl ActivityData {
    pub fn configure_active(&mut self, nodes: &[ProxyNode], trace: &ChainTrace) {
        // 保留每个 hop 的完整身份与服务器，包括没有服务器的 hop。
        let active_hops: Vec<_> = trace
            .hops
            .iter()
            .map(|hop| {
                let server = nodes
                    .iter()
                    .find(|node| node.id == hop.id)
                    .and_then(node_server)
                    .map(str::to_string);
                (hop.clone(), server)
            })
            .collect();
        let mut preferred_servers = Vec::new();
        for (_, server) in active_hops.iter().rev() {
            push_server(&mut preferred_servers, server.as_deref());
        }
        if self.active_hops != active_hops {
            for server in &preferred_servers {
                if !self.locations.contains_key(server) {
                    self.attempted.remove(server);
                }
            }
        }
        self.active_hops = active_hops;
        self.preferred_servers = preferred_servers;
    }

    pub fn next_geolocation_host(&mut self, candidates: &[String], now: u64) -> Option<String> {
        let host = self
            .preferred_servers
            .iter()
            .chain(candidates.iter())
            .find(|host| {
                if host.is_empty() || self.locations.contains_key(*host) {
                    return false;
                }
                let backoff = if self.preferred_servers.contains(host) {
                    PREFERRED_BACKOFF_MS
                } else {
                    ATTEMPT_BACKOFF_MS
                };
                self.attempted
                    .get(*host)
                    .map(|attempted| now.saturating_sub(*attempted) >= backoff)
                    .unwrap_or(true)
            })?
            .clone();
        // 选择与预留在同一把锁内完成；首次尝试在 now == 0 时也立即可选。
        self.attempted.insert(host.clone(), now);
        Some(host)
    }

    fn begin_run(&mut self, supported: bool) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.connections.clear();
        self.connected = false;
        self.enabled = true;
        self.supported = supported;
        self.resolving = false;
        self.generation
    }

    fn end_run(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.enabled = false;
        self.connected = false;
        self.resolving = false;
        self.connections.clear();
    }

    fn accepts_run(&self, generation: u64, stopped: bool, workers_valid: bool) -> bool {
        !stopped && workers_valid && self.enabled && self.generation == generation
    }

    // 生命周期输入由调用方在持锁后读取，纯数据测试不依赖全局 workers 状态。
    fn apply_poll(
        &mut self,
        generation: u64,
        stopped: bool,
        workers_valid: bool,
        connections: Option<Vec<CoreConnection>>,
    ) -> Option<bool> {
        if !self.accepts_run(generation, stopped, workers_valid) {
            return None;
        }
        let connected = connections.is_some();
        let mut connections = connections.unwrap_or_default();
        // 与 macOS 一致：按总流量降序，平局按 id。
        connections.sort_by(|a, b| {
            let left = a.download.saturating_add(a.upload);
            let right = b.download.saturating_add(b.upload);
            right.cmp(&left).then_with(|| a.id.cmp(&b.id))
        });
        let changed = self.connected != connected || self.connections != connections;
        self.connections = connections;
        self.connected = connected;
        Some(changed)
    }

    fn finish_geolocation(
        &mut self,
        generation: u64,
        stopped: bool,
        workers_valid: bool,
        host: String,
        location: Option<GeoLocation>,
    ) -> Option<bool> {
        if !self.accepts_run(generation, stopped, workers_valid) {
            return None;
        }
        let located = location.is_some();
        if let Some(location) = location {
            self.locations.insert(host, location);
        }
        self.resolving = false;
        Some(located)
    }
}

pub type SharedActivity = Arc<Mutex<ActivityData>>;

fn run_is_current(data: &SharedActivity, generation: u64, stop: &AtomicBool) -> bool {
    data.lock()
        .map(|guard| {
            guard.accepts_run(
                generation,
                stop.load(Ordering::SeqCst),
                crate::services::workers::valid(),
            )
        })
        .unwrap_or(false)
}

pub fn start(shared: &Shared, client: CoreClient, data: SharedActivity, stop: Arc<AtomicBool>) {
    let supported = client.kind.capabilities().connections;
    // 与 publish_route 保持相同锁顺序：先 Store，再 Activity。
    let Some((generation, ctx)) = crate::state::with_store(shared, |store| {
        if store.exiting || store.stopping {
            return None;
        }
        let nodes = store.visualization_nodes();
        let trace = store.active_trace.clone();
        let ctx = store.egui_ctx.clone();
        let mut guard = data.lock().ok()?;
        if stop.load(Ordering::SeqCst) || !crate::services::workers::valid() {
            return None;
        }
        guard.configure_active(&nodes, &trace);
        Some((guard.begin_run(supported), ctx))
    }) else {
        return;
    };

    // Xray 仅跳过连接轮询；定位与连接列表能力无关。
    if supported {
        let poll_data = Arc::clone(&data);
        let poll_client = client.clone();
        let poll_stop = Arc::clone(&stop);
        let poll_ctx = ctx.clone();
        crate::services::workers::spawn(move || {
            while run_is_current(&poll_data, generation, &poll_stop) {
                let connections = poll_client
                    .connections()
                    .ok()
                    .map(|snapshot| snapshot.connections);
                let changed = {
                    let Ok(mut guard) = poll_data.lock() else {
                        return;
                    };
                    // 请求返回后重新读取 stop 与 workers 状态，并在同一把锁内校验 run。
                    let Some(changed) = guard.apply_poll(
                        generation,
                        poll_stop.load(Ordering::SeqCst),
                        crate::services::workers::valid(),
                        connections,
                    ) else {
                        return;
                    };
                    changed
                };
                if changed {
                    if let Some(ctx) = &poll_ctx {
                        ctx.request_repaint();
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }

    let geo_shared = Arc::clone(shared);
    let geo_data = Arc::clone(&data);
    let geo_stop = Arc::clone(&stop);
    crate::services::workers::spawn(move || {
        while run_is_current(&geo_data, generation, &geo_stop) {
            let candidates = collect_candidates(&geo_shared, &geo_data);
            let now = crate::logic::health::now_ms();
            let target = {
                let Ok(mut guard) = geo_data.lock() else {
                    return;
                };
                if !guard.accepts_run(
                    generation,
                    geo_stop.load(Ordering::SeqCst),
                    crate::services::workers::valid(),
                ) {
                    return;
                }
                let target = guard.next_geolocation_host(&candidates, now);
                if target.is_some() {
                    guard.resolving = true;
                }
                target
            };
            if let Some(host) = target {
                if !run_is_current(&geo_data, generation, &geo_stop) {
                    return;
                }
                let location = geoip::location(&host);
                let located = {
                    let Ok(mut guard) = geo_data.lock() else {
                        return;
                    };
                    let Some(located) = guard.finish_geolocation(
                        generation,
                        geo_stop.load(Ordering::SeqCst),
                        crate::services::workers::valid(),
                        host,
                        location,
                    ) else {
                        return;
                    };
                    located
                };
                if located {
                    if let Some(ctx) = &ctx {
                        ctx.request_repaint();
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(1200));
        }
        // geoip::location 已持久化成功结果；过期线程不再额外落盘。
    });
}

fn node_server(node: &ProxyNode) -> Option<&str> {
    node.payload.get("server").and_then(|value| value.as_str())
}

fn push_server(candidates: &mut Vec<String>, server: Option<&str>) {
    if let Some(server) = server {
        if !server.is_empty() && !candidates.iter().any(|candidate| candidate == server) {
            candidates.push(server.to_string());
        }
    }
}

/// 当前链路出口优先，其次为活跃连接的路径与目标，最后为其余已知节点。
fn candidate_hosts(
    nodes: &[ProxyNode],
    trace: &ChainTrace,
    connections: &[CoreConnection],
) -> Vec<String> {
    let index: HashMap<String, &ProxyNode> =
        nodes.iter().map(|node| (node.name.clone(), node)).collect();
    let mut candidates = Vec::new();
    for hop in trace.hops.iter().rev() {
        let server = nodes
            .iter()
            .find(|node| node.id == hop.id)
            .and_then(node_server);
        push_server(&mut candidates, server);
    }
    for connection in connections.iter().take(80) {
        for node in connection.route_nodes(&index).into_iter().rev() {
            push_server(&mut candidates, node_server(node));
        }
        push_server(
            &mut candidates,
            connection.metadata.geolocation_key().as_deref(),
        );
    }
    for node in nodes {
        push_server(&mut candidates, node_server(node));
    }
    candidates
}

fn collect_candidates(shared: &Shared, data: &SharedActivity) -> Vec<String> {
    let (nodes, trace) = crate::state::with_store(shared, |store| {
        (store.visualization_nodes(), store.active_trace.clone())
    });
    // Store 锁已释放；持有 Activity 锁期间不再取 Store。
    if let Ok(guard) = data.lock() {
        candidate_hosts(&nodes, &trace, &guard.connections)
    } else {
        candidate_hosts(&nodes, &trace, &[])
    }
}

pub fn stop(data: &SharedActivity) {
    if let Ok(mut guard) = data.lock() {
        guard.end_run();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::model::ConnectionMetadata;
    use serde_json::{Map, Value};

    fn node(id: &str, server: &str) -> ProxyNode {
        let mut payload = Map::new();
        payload.insert("server".into(), Value::String(server.into()));
        ProxyNode {
            id: id.into(),
            name: id.into(),
            source_id: "source".into(),
            payload,
            exit_name: String::new(),
        }
    }

    fn trace(nodes: &[ProxyNode]) -> ChainTrace {
        ChainTrace {
            hops: nodes
                .iter()
                .map(|node| ChainHop {
                    id: node.id.clone(),
                    name: node.name.clone(),
                    source_id: node.source_id.clone(),
                    protocol_name: node.protocol_name(),
                })
                .collect(),
            complete: !nodes.is_empty(),
        }
    }

    fn location() -> GeoLocation {
        GeoLocation {
            latitude: 0.0,
            longitude: 0.0,
            country: String::new(),
            city: String::new(),
        }
    }

    fn connection(id: &str, download: u64) -> CoreConnection {
        CoreConnection {
            id: id.into(),
            metadata: ConnectionMetadata::default(),
            upload: 0,
            download,
            chains: Vec::new(),
            rule: None,
        }
    }

    #[test]
    fn active_exit_precedes_entry_and_other_candidates() {
        let active = vec![node("entry", "entry.example"), node("exit", "exit.example")];
        let nodes = vec![
            node("other", "other.example"),
            active[0].clone(),
            active[1].clone(),
        ];
        let trace = trace(&active);
        let candidates = candidate_hosts(&nodes, &trace, &[]);
        assert_eq!(
            candidates,
            vec!["exit.example", "entry.example", "other.example"]
        );
        let mut data = ActivityData::default();
        data.configure_active(&nodes, &trace);
        data.begin_run(false);
        assert!(data.enabled && !data.supported);
        let fallback = vec!["other.example".into()];
        assert_eq!(
            data.next_geolocation_host(&fallback, 0).as_deref(),
            Some("exit.example")
        );
        assert_eq!(
            data.next_geolocation_host(&fallback, 0).as_deref(),
            Some("entry.example")
        );
        assert_eq!(
            data.next_geolocation_host(&fallback, 0).as_deref(),
            Some("other.example")
        );
    }

    #[test]
    fn preferred_retry_includes_exact_ten_second_boundary() {
        let nodes = vec![node("exit", "exit.example")];
        let mut data = ActivityData::default();
        data.configure_active(&nodes, &trace(&nodes));
        assert_eq!(
            data.next_geolocation_host(&[], 0).as_deref(),
            Some("exit.example")
        );
        assert_eq!(data.next_geolocation_host(&[], 9_999), None);
        assert_eq!(
            data.next_geolocation_host(&[], 10_000).as_deref(),
            Some("exit.example")
        );
    }

    #[test]
    fn other_retry_includes_exact_five_minute_boundary() {
        let candidates = vec!["other.example".into()];
        let mut data = ActivityData::default();
        assert_eq!(
            data.next_geolocation_host(&candidates, 0).as_deref(),
            Some("other.example")
        );
        assert_eq!(data.next_geolocation_host(&candidates, 299_999), None);
        assert_eq!(
            data.next_geolocation_host(&candidates, 300_000).as_deref(),
            Some("other.example")
        );
    }

    #[test]
    fn identity_switch_resets_only_new_unknown_preferred_once() {
        let old = vec![node("old", "old.example")];
        let new = vec![node("new", "new.example")];
        let mut data = ActivityData::default();
        data.configure_active(&old, &trace(&old));
        for server in ["old.example", "new.example", "other.example"] {
            data.attempted.insert(server.into(), 0);
        }
        data.configure_active(&new, &trace(&new));
        assert!(!data.attempted.contains_key("new.example"));
        assert_eq!(data.attempted.get("old.example"), Some(&0));
        assert_eq!(data.attempted.get("other.example"), Some(&0));
        assert_eq!(
            data.next_geolocation_host(&[], 1).as_deref(),
            Some("new.example")
        );
        data.configure_active(&new, &trace(&new));
        assert_eq!(data.next_geolocation_host(&[], 2), None);
    }

    #[test]
    fn shared_server_with_different_ids_resets_retry_once() {
        let first = vec![node("first", "shared.example")];
        let second = vec![node("second", "shared.example")];
        let mut data = ActivityData::default();
        data.configure_active(&first, &trace(&first));
        assert!(data.next_geolocation_host(&[], 0).is_some());
        assert_eq!(data.next_geolocation_host(&[], 1), None);
        data.configure_active(&second, &trace(&second));
        assert_eq!(
            data.next_geolocation_host(&[], 1).as_deref(),
            Some("shared.example")
        );
        data.configure_active(&second, &trace(&second));
        assert_eq!(data.next_geolocation_host(&[], 2), None);
    }

    #[test]
    fn same_identity_with_changed_server_resets_retry_once() {
        let old = vec![node("exit", "old.example")];
        let new = vec![node("exit", "new.example")];
        let trace = trace(&old);
        let mut data = ActivityData::default();
        data.configure_active(&old, &trace);
        data.attempted.insert("new.example".into(), 0);
        data.configure_active(&new, &trace);
        assert_eq!(
            data.next_geolocation_host(&[], 1).as_deref(),
            Some("new.example")
        );
        data.configure_active(&new, &trace);
        assert_eq!(data.next_geolocation_host(&[], 2), None);
    }

    #[test]
    fn complete_hop_identity_including_serverless_hops_is_tracked() {
        let mut nodes = vec![node("entry", ""), node("exit", "exit.example")];
        nodes[0].payload.remove("server");
        let mut trace = trace(&nodes);
        let mut data = ActivityData::default();
        data.configure_active(&nodes, &trace);
        assert_eq!(data.active_hops.len(), 2);
        assert_eq!(data.active_hops[0].1, None);
        assert!(data.next_geolocation_host(&[], 0).is_some());
        trace.hops[0].source_id = "changed-source".into();
        data.configure_active(&nodes, &trace);
        assert!(data.next_geolocation_host(&[], 1).is_some());
        trace.hops[0].name = "changed-name".into();
        data.configure_active(&nodes, &trace);
        assert!(data.next_geolocation_host(&[], 2).is_some());
        trace.hops[0].protocol_name = "changed-protocol".into();
        data.configure_active(&nodes, &trace);
        assert!(data.next_geolocation_host(&[], 3).is_some());
        trace.complete = false;
        data.configure_active(&nodes, &trace);
        assert_eq!(data.next_geolocation_host(&[], 4), None);
    }

    #[test]
    fn stop_and_restart_reject_old_poll_and_geolocation_results() {
        let shared = Arc::new(Mutex::new(ActivityData::default()));
        let old_generation = {
            let mut data = shared.lock().unwrap();
            let generation = data.begin_run(true);
            assert_eq!(
                data.apply_poll(generation, false, true, Some(vec![connection("old", 1)])),
                Some(true)
            );
            data.resolving = true;
            generation
        };
        stop(&shared);
        let mut data = shared.lock().unwrap();
        assert_ne!(data.generation, old_generation);
        assert!(!data.enabled && !data.connected && !data.resolving);
        assert!(data.connections.is_empty());
        assert_eq!(
            data.apply_poll(
                old_generation,
                false,
                true,
                Some(vec![connection("late", 2)])
            ),
            None
        );
        assert_eq!(
            data.finish_geolocation(
                old_generation,
                false,
                true,
                "old.example".into(),
                Some(location()),
            ),
            None
        );
        let generation = data.begin_run(true);
        assert_ne!(generation, old_generation);
        assert_eq!(
            data.apply_poll(generation, false, true, Some(vec![connection("new", 3)])),
            Some(true)
        );
        data.resolving = true;
        assert_eq!(data.apply_poll(old_generation, false, true, None), None);
        assert_eq!(
            data.apply_poll(
                old_generation,
                false,
                true,
                Some(vec![connection("late", 4)])
            ),
            None
        );
        assert_eq!(
            data.finish_geolocation(
                old_generation,
                false,
                true,
                "old.example".into(),
                Some(location()),
            ),
            None
        );
        assert_eq!(
            data.finish_geolocation(old_generation, false, true, "old.example".into(), None),
            None
        );
        assert_eq!(data.connections[0].id, "new");
        assert!(data.connected && data.resolving);
        assert!(data.locations.is_empty());
        assert_eq!(
            data.finish_geolocation(
                generation,
                false,
                true,
                "new.example".into(),
                Some(location()),
            ),
            Some(true)
        );
        assert!(!data.resolving);
        assert!(data.locations.contains_key("new.example"));
    }

    #[test]
    fn stopped_disabled_or_invalid_workers_reject_results() {
        let mut data = ActivityData::default();
        let generation = data.begin_run(true);
        data.resolving = true;
        for (stopped, workers_valid) in [(true, true), (false, false)] {
            assert_eq!(
                data.apply_poll(
                    generation,
                    stopped,
                    workers_valid,
                    Some(vec![connection("late", 1)]),
                ),
                None
            );
            assert_eq!(
                data.finish_geolocation(
                    generation,
                    stopped,
                    workers_valid,
                    "late.example".into(),
                    Some(location()),
                ),
                None
            );
        }
        data.enabled = false;
        assert_eq!(data.apply_poll(generation, false, true, None), None);
        assert_eq!(
            data.finish_geolocation(
                generation,
                false,
                true,
                "late.example".into(),
                Some(location()),
            ),
            None
        );
        assert!(data.connections.is_empty() && data.locations.is_empty());
        assert!(data.resolving);
    }

    #[test]
    fn cached_locations_survive_switch_and_runs_without_retry() {
        let first = vec![node("first", "cached.example")];
        let second = vec![node("second", "cached.example")];
        let candidates = vec!["cached.example".into()];
        let mut data = ActivityData::default();
        data.configure_active(&first, &trace(&first));
        data.locations.insert("cached.example".into(), location());
        data.attempted.insert("cached.example".into(), 7);
        data.configure_active(&second, &trace(&second));
        assert_eq!(data.attempted.get("cached.example"), Some(&7));
        data.begin_run(false);
        assert!(data.enabled && !data.supported);
        assert_eq!(data.next_geolocation_host(&candidates, u64::MAX), None);
        data.end_run();
        data.begin_run(true);
        assert_eq!(data.next_geolocation_host(&candidates, u64::MAX), None);
        assert_eq!(data.locations.get("cached.example"), Some(&location()));
        assert_eq!(data.attempted.get("cached.example"), Some(&7));
    }

    #[test]
    fn poll_reports_changes_and_sorts_connections() {
        let mut data = ActivityData::default();
        let generation = data.begin_run(true);
        let connections = vec![connection("z", 9), connection("b", 10), connection("a", 10)];
        assert_eq!(
            data.apply_poll(generation, false, true, Some(connections.clone())),
            Some(true)
        );
        assert_eq!(
            data.connections
                .iter()
                .map(|connection| connection.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "z"]
        );
        assert_eq!(
            data.apply_poll(generation, false, true, Some(connections)),
            Some(false)
        );
        assert_eq!(data.apply_poll(generation, false, true, None), Some(true));
        assert_eq!(data.apply_poll(generation, false, true, None), Some(false));
        assert!(!data.connected && data.connections.is_empty());
    }
}
