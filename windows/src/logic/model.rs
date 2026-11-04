use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub const PROXY_PORT: u16 = 17890;
pub const PROBE_PORT: u16 = 17891;
pub const AGGREGATE_PORT: u16 = 17892;
pub const CONTROLLER_PORT: u16 = 19090;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub url: String,
}

impl Source {
    /// 空地址表示从本地导入的离线快照。
    pub fn is_local(&self) -> bool {
        self.url.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct ProxyNode {
    pub id: String,
    pub name: String,
    pub source_id: String,
    pub payload: Map<String, Value>,
}

impl ProxyNode {
    pub fn protocol_name(&self) -> String {
        self.payload
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("proxy")
            .to_uppercase()
    }

    pub fn short_name(&self) -> String {
        self.name
            .split(" · ")
            .next()
            .unwrap_or(&self.name)
            .to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoutingMode {
    LowLatency,
    HighSpeed,
    Ai,
}

impl RoutingMode {
    pub const ALL: [RoutingMode; 3] = [RoutingMode::LowLatency, RoutingMode::HighSpeed, RoutingMode::Ai];

    pub fn title(self) -> &'static str {
        match self {
            RoutingMode::LowLatency => "低延迟链路",
            RoutingMode::HighSpeed => "高速链路",
            RoutingMode::Ai => "AI 链路",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            RoutingMode::LowLatency => "优先选择实测 HTTPS 延迟较低、出网正常的线路。",
            RoutingMode::HighSpeed => "对最多 3 个候选各下载 1 MiB，按实测吞吐选择；每轮约消耗 3 MiB 测速流量。",
            RoutingMode::Ai => "线路必须通过 OpenAI API 可达性检查；不代表 ChatGPT 网页登录或账号权限。",
        }
    }
}

/// 流量模式：进入内核的流量如何出网。与 macOS 版 TrafficMode 保持一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrafficMode {
    Rule,
    Global,
    Direct,
}

impl TrafficMode {
    pub const ALL: [TrafficMode; 3] = [TrafficMode::Rule, TrafficMode::Global, TrafficMode::Direct];

    /// 与 Mihomo `/configs` 的 `mode` 字段一致。
    pub fn raw(self) -> &'static str {
        match self {
            TrafficMode::Rule => "rule",
            TrafficMode::Global => "global",
            TrafficMode::Direct => "direct",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            TrafficMode::Rule => "规则代理",
            TrafficMode::Global => "全局代理",
            TrafficMode::Direct => "直连",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            TrafficMode::Rule => "按内置规则分流：本机与局域网地址直连，其余流量走当前代理线路。",
            TrafficMode::Global => "进入内核的业务流量全部走当前代理线路，跳过分流与局域网例外。",
            TrafficMode::Direct => "进入内核的业务流量全部直连，不使用代理节点；系统代理和 TUN 可继续开启。",
        }
    }

    pub fn uses_proxy(self) -> bool {
        self != TrafficMode::Direct
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ChainOptions {
    pub enabled: bool,
    pub entry_ids: BTreeSet<String>,
    pub exit_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_true")]
    pub automatic: bool,
    #[serde(default)]
    pub explore_multihop: Option<bool>,
    #[serde(default)]
    pub routing_mode: Option<RoutingMode>,
    #[serde(default = "default_traffic_mode")]
    pub traffic_mode: TrafficMode,
    #[serde(default)]
    pub chain: Option<ChainOptions>,
    #[serde(default = "default_secret")]
    pub secret: String,
    #[serde(default = "default_secret")]
    pub subscription_token: String,
}

fn default_interval() -> u64 {
    60
}

fn default_true() -> bool {
    true
}

fn default_traffic_mode() -> TrafficMode {
    TrafficMode::Rule
}

fn default_secret() -> String {
    crate::logic::model::random_hex(32)
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            interval: 60,
            automatic: true,
            explore_multihop: Some(false),
            routing_mode: Some(RoutingMode::LowLatency),
            traffic_mode: TrafficMode::Rule,
            chain: None,
            secret: default_secret(),
            subscription_token: default_secret(),
        }
    }
}

impl Preferences {
    pub fn mode(&self) -> RoutingMode {
        self.routing_mode.unwrap_or(RoutingMode::LowLatency)
    }
}

/// 不依赖外部 RNG 的随机十六进制串：用于本机控制密钥与订阅令牌。
pub fn random_hex(bytes: usize) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut out = String::with_capacity(bytes * 2);
    let mut counter: u64 = 0;
    while out.len() < bytes * 2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(counter);
        counter += 1;
        if let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) {
            hasher.write_u64(now.as_nanos() as u64);
        }
        hasher.write_u32(std::process::id());
        let value = hasher.finish();
        out.push_str(&format!("{value:016x}"));
    }
    out.truncate(bytes * 2);
    out
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeLatency {
    pub milliseconds: Option<i64>,
    #[serde(default)]
    pub checked: u64,
    #[serde(default)]
    pub failure: Option<String>,
}

impl NodeLatency {
    pub fn new(milliseconds: Option<i64>, failure: Option<String>) -> Self {
        Self {
            milliseconds,
            checked: crate::logic::health::now_ms(),
            failure,
        }
    }

    pub fn reachable(&self) -> bool {
        self.milliseconds.map(|v| v >= 0).unwrap_or(false)
    }

    pub fn is_fresh(&self, now: u64, lifetime_ms: u64) -> bool {
        now >= self.checked && now - self.checked <= lifetime_ms
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThroughputSample {
    pub bytes_per_second: f64,
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub checked: u64,
}

impl ThroughputSample {
    pub fn megabits_per_second(&self) -> f64 {
        self.bytes_per_second * 8.0 / 1_000_000.0
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LinkMeasurements {
    #[serde(default)]
    pub throughput: Option<ThroughputSample>,
    #[serde(default)]
    pub openai_reachable: Option<bool>,
    #[serde(default)]
    pub openai_checked: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Healthy,
    Blocked,
    Limited,
    Unknown,
    Offline,
}

impl Health {
    pub fn label(self) -> &'static str {
        match self {
            Health::Healthy => "healthy",
            Health::Blocked => "blocked",
            Health::Limited => "limited",
            Health::Unknown => "unknown",
            Health::Offline => "offline",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProbeResult {
    pub health: Health,
    pub milliseconds: i64,
    pub detail: String,
}

impl ProbeResult {
    pub fn new(health: Health, milliseconds: i64, detail: String) -> Self {
        Self { health, milliseconds, detail }
    }
}

#[derive(Clone, Debug)]
pub struct NodeStatus {
    pub result: ProbeResult,
    pub checked: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeSortOrder {
    Latency,
    Name,
    Original,
}

impl NodeSortOrder {
    pub const ALL: [NodeSortOrder; 3] = [NodeSortOrder::Latency, NodeSortOrder::Name, NodeSortOrder::Original];

    pub fn title(self) -> &'static str {
        match self {
            NodeSortOrder::Latency => "延迟从低到高",
            NodeSortOrder::Name => "节点名称",
            NodeSortOrder::Original => "原始顺序",
        }
    }
}
