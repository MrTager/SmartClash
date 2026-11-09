pub mod actions;
pub mod activity;
pub mod traffic;
mod tun_transition;

use crate::logic::chain::ChainTrace;
use crate::logic::dynamic_router::RoutePlan;
use crate::logic::ranking::NodeRanking;
use crate::logic::model::{
    LinkMeasurements, NodeLatency, NodeStatus, Preferences, ProbeResult, ProxyNode, RoutingMode,
};
use crate::services::aggregate_server::AggregateServer;
use crate::services::core_process::CoreProcess;
use crate::services::paths;
use crate::services::subscription_service;
use crate::services::system_proxy::SystemProxy;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Shared = Arc<Mutex<Store>>;

pub struct Store {
    pub preferences: Preferences,
    pub nodes: Vec<ProxyNode>,
    pub latencies: HashMap<String, NodeLatency>,
    pub measurements: HashMap<String, LinkMeasurements>,
    pub statuses: HashMap<String, NodeStatus>,
    pub selected: String,
    pub running: bool,
    pub tun: bool,
    pub system_proxy: bool,
    pub busy: bool,
    pub stopping: bool,
    pub exiting: bool,
    pub shutdown_running: bool,
    pub shutdown_done: bool,
    pub shutdown_error: Option<String>,
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub recovery: bool,
    pub subscription_ready: bool,
    pub is_testing_latency: bool,
    pub latency_completed: usize,
    pub latency_total: usize,
    pub testing_ids: HashSet<String>,
    pub internet_status: String,
    pub openai_status: String,
    pub active_mode: Option<RoutingMode>,
    pub active_trace: ChainTrace,
    pub standby_ready: usize,
    pub routing_status: String,
    pub rule_status: String,
    pub dynamic_mode: bool,
    pub route_plan: RoutePlan,
    pub core: CoreProcess,
    pub aggregate: AggregateServer,
    pub lifecycle: Arc<Mutex<()>>,
    pub epoch: u64,
    pub traffic: Arc<Mutex<traffic::TrafficData>>,
    pub activity: activity::SharedActivity,
    pub activity_stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub monitor_stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub latency_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub last_update: u64,
    pub last_switch: u64,
    pub failures: u32,
    pub scan_offset: usize,
    pub search_round: u32,
    pub last_openai_probe: u64,
    pub last_openai_failover: u64,
    pub traffic_stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub dirty_traffic: bool,
    pub egui_ctx: Option<egui::Context>,
}

impl Store {
    pub fn new() -> Result<Self, String> {
        let root = paths::ensure_root()?;
        let settings = root.join("preferences.json");
        let mut preferences = match std::fs::read(&settings)
            .ok()
            .and_then(|data| serde_json::from_slice::<Preferences>(&data).ok())
        {
            Some(value) => value,
            None => Preferences::default(),
        };
        preferences.interval = preferences.interval.clamp(1, 3600);
        // 代理端口非法或与保留端口冲突时回退默认值，避免历史脏数据卡死启动。
        if crate::logic::model::validate_proxy_port(preferences.proxy_port).is_err() {
            preferences.proxy_port = crate::logic::model::DEFAULT_PROXY_PORT;
        }
        crate::logic::model::set_proxy_port(preferences.proxy_port);
        let _ = paths::write_private(
            &settings,
            &serde_json::to_vec_pretty(&preferences).unwrap_or_default(),
        );

        let nodes = subscription_service::cached(&preferences.sources);
        let node_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();

        let mut latencies: HashMap<String, NodeLatency> =
            std::fs::read(root.join("latencies.json"))
                .ok()
                .and_then(|data| serde_json::from_slice(&data).ok())
                .unwrap_or_default();
        latencies.retain(|id, _| node_ids.contains(id));

        let measurements: HashMap<String, LinkMeasurements> =
            std::fs::read(root.join("link-measurements.json"))
                .ok()
                .and_then(|data| serde_json::from_slice(&data).ok())
                .unwrap_or_default();

        let aggregate = AggregateServer::new(&preferences.subscription_token);
        if !nodes.is_empty() {
            let _ = aggregate.update(&nodes);
        }
        let subscription_ready = aggregate.start().is_ok();
        let core_kind = preferences.kernel;

        let mut store = Self {
            preferences,
            nodes,
            latencies,
            measurements,
            statuses: HashMap::new(),
            selected: String::new(),
            running: false,
            tun: false,
            system_proxy: SystemProxy::has_backup(),
            busy: false,
            stopping: false,
            exiting: false,
            shutdown_running: false,
            shutdown_done: false,
            shutdown_error: None,
            error: None,
            logs: Vec::new(),
            recovery: false,
            subscription_ready,
            is_testing_latency: false,
            latency_completed: 0,
            latency_total: 0,
            testing_ids: HashSet::new(),
            internet_status: "未验证联网".into(),
            openai_status: "OpenAI 尚未检测".into(),
            active_mode: None,
            active_trace: ChainTrace::default(),
            standby_ready: 0,
            routing_status: "尚未打通".into(),
            rule_status: "尚未手动更新，内核运行后每小时自动刷新".into(),
            dynamic_mode: false,
            route_plan: RoutePlan::default(),
            core: CoreProcess::new(core_kind),
            aggregate,
            lifecycle: Arc::new(Mutex::new(())),
            epoch: 0,
            traffic: Arc::new(Mutex::new(traffic::TrafficData::load())),
            activity: Arc::new(Mutex::new(activity::ActivityData::default())),
            activity_stop: None,
            monitor_stop: None,
            latency_cancel: None,
            last_update: crate::logic::health::now_ms(),
            last_switch: 0,
            failures: 0,
            scan_offset: 0,
            search_round: 0,
            last_openai_probe: 0,
            last_openai_failover: 0,
            traffic_stop: None,
            dirty_traffic: false,
            egui_ctx: None,
        };
        store.recovery = SystemProxy::has_backup() || store.core.recovery_needed();
        if store.recovery {
            store.log("发现上次运行的恢复记录，请点击“恢复网络”");
        }
        Ok(store)
    }

    pub fn log(&mut self, message: &str) {
        if self.logs.len() >= 300 {
            self.logs.pop();
        }
        self.logs
            .insert(0, format!("{}  {}", paths::time_text(), message));
    }

    pub fn save_preferences(&self) {
        if let Ok(data) = serde_json::to_vec_pretty(&self.preferences) {
            let _ = paths::write_private(&paths::data_root().join("preferences.json"), &data);
        }
    }

    pub fn save_latencies(&self) {
        if let Ok(data) = serde_json::to_vec(&self.latencies) {
            let _ = paths::write_private(&paths::data_root().join("latencies.json"), &data);
        }
    }

    pub fn save_measurements(&self) {
        if let Ok(data) = serde_json::to_vec(&self.measurements) {
            let _ = paths::write_private(&paths::data_root().join("link-measurements.json"), &data);
        }
    }

    pub fn can_change_sources(&self) -> bool {
        !self.exiting && !self.running && !self.busy && !self.recovery && !self.is_testing_latency
    }

    pub fn route_nodes(&self) -> Vec<ProxyNode> {
        if self.route_plan.candidates.is_empty() {
            self.nodes.clone()
        } else {
            self.route_plan.candidates.clone()
        }
    }

    pub fn node_list_nodes(&self) -> Vec<ProxyNode> {
        let ids: HashSet<&String> = self.nodes.iter().map(|n| &n.id).collect();
        let mut list = self.nodes.clone();
        for node in &self.route_plan.candidates {
            if !ids.contains(&node.id) {
                list.push(node.clone());
            }
        }
        list
    }

    /// 地球与实时连接需要完整定义，不能遗漏动态链路的中间跳与生成出口。
    pub fn visualization_nodes(&self) -> Vec<ProxyNode> {
        let mut seen = HashSet::new();
        self.nodes
            .iter()
            .chain(self.route_plan.definitions.iter())
            .chain(self.route_plan.candidates.iter())
            .filter(|node| seen.insert(node.id.clone()))
            .cloned()
            .collect()
    }

    pub fn subscription_url(&self) -> String {
        self.aggregate.address()
    }

    pub fn publish_route(&mut self) {
        let known = self.visualization_nodes();
        let trace = ChainTrace::resolve(
            if self.running && self.preferences.traffic_mode.uses_proxy() {
                &self.selected
            } else {
                ""
            },
            &self.nodes,
            &known,
        );
        if let Ok(mut activity) = self.activity.lock() {
            activity.configure_active(&known, &trace);
        }
        self.active_trace = trace;
        self.standby_ready = self.fresh_standbys().len();
    }

    pub fn fresh_standbys(&self) -> Vec<ProxyNode> {
        let now = crate::logic::health::now_ms();
        let mut list: Vec<ProxyNode> = self
            .route_plan
            .candidates
            .iter()
            .filter(|node| {
                node.name != self.selected
                    && self
                        .statuses
                        .get(&node.id)
                        .map(|status| {
                            crate::logic::health::StandbyPolicy::usable(
                                &status.result,
                                status.checked,
                                now,
                                15_000,
                            )
                        })
                        .unwrap_or(false)
            })
            .cloned()
            .collect();
        list.sort_by(|a, b| {
            let region = NodeRanking::is_hong_kong(a).cmp(&NodeRanking::is_hong_kong(b));
            if region != std::cmp::Ordering::Equal {
                return region;
            }
            if self.active_mode == Some(RoutingMode::HighSpeed) {
                let left = self
                    .measurements
                    .get(&a.id)
                    .and_then(|m| m.throughput.as_ref())
                    .map(|t| t.bytes_per_second)
                    .unwrap_or(0.0);
                let right = self
                    .measurements
                    .get(&b.id)
                    .and_then(|m| m.throughput.as_ref())
                    .map(|t| t.bytes_per_second)
                    .unwrap_or(0.0);
                if left != right {
                    return right
                        .partial_cmp(&left)
                        .unwrap_or(std::cmp::Ordering::Equal);
                }
            }
            let lm = self
                .statuses
                .get(&a.id)
                .map(|s| s.result.milliseconds)
                .unwrap_or(i64::MAX);
            let rm = self
                .statuses
                .get(&b.id)
                .map(|s| s.result.milliseconds)
                .unwrap_or(i64::MAX);
            lm.cmp(&rm)
        });
        list
    }

    pub fn set_status(&mut self, id: &str, result: ProbeResult) {
        self.statuses.insert(
            id.to_string(),
            NodeStatus {
                result,
                checked: crate::logic::health::now_ms(),
            },
        );
    }
}

pub fn with_store<R>(shared: &Shared, f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = shared.lock().unwrap();
    f(&mut guard)
}

pub fn request_repaint(ctx: &egui::Context) {
    ctx.request_repaint_after(Duration::from_millis(200));
}
