use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU16, Ordering};

pub const DEFAULT_PROXY_PORT: u16 = 7890;
pub const PROBE_PORT: u16 = 7891;
pub const AGGREGATE_PORT: u16 = 7892;
pub const CONTROLLER_PORT: u16 = 9090;

/// 当前生效的本地混合代理端口（HTTP + SOCKS）。默认 7890，可在设置中修改。
static ACTIVE_PROXY_PORT: AtomicU16 = AtomicU16::new(DEFAULT_PROXY_PORT);

pub fn proxy_port() -> u16 {
    ACTIVE_PROXY_PORT.load(Ordering::SeqCst)
}

pub fn set_proxy_port(port: u16) {
    ACTIVE_PROXY_PORT.store(port, Ordering::SeqCst);
}

/// 校验用户输入的代理端口：合法范围 1024–65535，且不得与内部保留端口冲突。
pub fn validate_proxy_port(port: u16) -> Result<(), String> {
    if !(1024..=65535).contains(&port) {
        return Err("代理端口需在 1024–65535 之间".into());
    }
    let reserved: [(u16, &str); 5] = [
        (PROBE_PORT, "本地探测端口"),
        (AGGREGATE_PORT, "订阅聚合端口"),
        (CONTROLLER_PORT, "内核控制端口"),
        (crate::logic::kernel::XRAY_SOCKS_PORT, "Xray SOCKS 端口"),
        (crate::logic::kernel::XRAY_API_PORT, "Xray 控制端口"),
    ];
    if let Some((_, label)) = reserved.iter().find(|(value, _)| *value == port) {
        return Err(format!("端口 {port} 与{label}冲突，请换一个"));
    }
    Ok(())
}

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
    /// Original exit label; never serialized into a kernel configuration.
    pub exit_name: String,
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
    pub const ALL: [RoutingMode; 3] = [
        RoutingMode::LowLatency,
        RoutingMode::HighSpeed,
        RoutingMode::Ai,
    ];

    pub fn title(self) -> &'static str {
        match self {
            RoutingMode::LowLatency => "低延迟链路",
            RoutingMode::HighSpeed => "高速链路",
            RoutingMode::Ai => "AI 链路",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            RoutingMode::LowLatency => "自动选路优先非中国香港节点，同组按实测 HTTPS 延迟选择；无合适线路时回退。",
            RoutingMode::HighSpeed => "优先非中国香港节点，按实测吞吐选择；每组最多测 3 个候选、各 1 MiB，含回退最多约 6 MiB。",
            RoutingMode::Ai => "优先非中国香港节点，线路仍须通过 OpenAI API 可达性检查；不代表 ChatGPT 网页登录或账号权限。",
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
            TrafficMode::Direct => {
                "进入内核的业务流量全部直连，不使用代理节点；系统代理和 TUN 可继续开启。"
            }
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
    /// 当前使用的代理内核；旧配置缺省为 Mihomo。
    #[serde(default)]
    pub kernel: crate::logic::kernel::KernelKind,
    /// 本地混合代理端口（HTTP + SOCKS）；旧配置缺省 7890。
    #[serde(default = "default_proxy_port")]
    pub proxy_port: u16,
}

fn default_interval() -> u64 {
    10
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

fn default_proxy_port() -> u16 {
    DEFAULT_PROXY_PORT
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            interval: 10,
            automatic: true,
            explore_multihop: Some(false),
            routing_mode: Some(RoutingMode::LowLatency),
            traffic_mode: TrafficMode::Rule,
            chain: None,
            secret: default_secret(),
            subscription_token: default_secret(),
            kernel: crate::logic::kernel::KernelKind::Mihomo,
            proxy_port: DEFAULT_PROXY_PORT,
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
        Self {
            health,
            milliseconds,
            detail,
        }
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
    pub const ALL: [NodeSortOrder; 3] = [
        NodeSortOrder::Latency,
        NodeSortOrder::Name,
        NodeSortOrder::Original,
    ];

    pub fn title(self) -> &'static str {
        match self {
            NodeSortOrder::Latency => "延迟从低到高",
            NodeSortOrder::Name => "节点名称",
            NodeSortOrder::Original => "原始顺序",
        }
    }
}

// ---------------------------------------------------------------- 连接快照（与 macOS 版 ProxyLogic 对齐）

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeoLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub country: String,
    #[serde(default)]
    pub city: String,
}

impl GeoLocation {
    pub fn label(&self) -> String {
        if self.city.is_empty() {
            self.country.clone()
        } else {
            format!("{}, {}", self.city, self.country)
        }
    }

    pub fn is_valid(&self) -> bool {
        self.latitude.is_finite()
            && self.longitude.is_finite()
            && (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectionMetadata {
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default, rename = "destinationIP")]
    pub destination_ip: Option<String>,
    #[serde(default, rename = "destinationPort")]
    pub destination_port: Option<String>,
    #[serde(default)]
    pub process: Option<String>,
}

impl ConnectionMetadata {
    pub fn display_destination(&self) -> String {
        if let Some(host) = &self.host {
            if !host.is_empty() {
                return host.clone();
            }
        }
        if let Some(ip) = &self.destination_ip {
            if !ip.is_empty() {
                return ip.clone();
            }
        }
        "未知目标".into()
    }

    /// 只有公网 IP 才参与定位；内网地址不发送给任何服务。
    pub fn public_destination_ip(&self) -> Option<&str> {
        self.destination_ip
            .as_deref()
            .filter(|ip| crate::services::geoip::is_public_ip(ip))
    }

    /// 定位键：优先公网目标 IP，缺失时用请求域名（由 DNS 估算位置）。
    pub fn geolocation_key(&self) -> Option<String> {
        if let Some(ip) = self.public_destination_ip() {
            return Some(ip.to_string());
        }
        self.host
            .as_deref()
            .filter(|host| !host.is_empty())
            .map(|host| host.to_lowercase())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoreConnection {
    pub id: String,
    #[serde(default)]
    pub metadata: ConnectionMetadata,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: Option<String>,
}

impl CoreConnection {
    pub fn path_label(&self) -> String {
        if self.chains.iter().any(|c| c == "DIRECT") {
            return "直连".into();
        }
        if self
            .chains
            .iter()
            .any(|c| c == "REJECT" || c == "REJECT-DROP")
        {
            return "已拦截".into();
        }
        self.chains
            .iter()
            .filter(|c| !matches!(c.as_str(), "SMART" | "PROBE" | "GLOBAL"))
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join(" → ")
    }

    /// 连接自身的链路在出口切换后仍保留旧路由；永远不要用当前选中节点推断。
    pub fn route_nodes<'a>(
        &self,
        index: &std::collections::HashMap<String, &'a ProxyNode>,
    ) -> Vec<&'a ProxyNode> {
        if self
            .chains
            .iter()
            .any(|c| c == "DIRECT" || c == "REJECT" || c == "REJECT-DROP")
        {
            return Vec::new();
        }
        let Some(exit) = self.chains.iter().find(|name| index.contains_key(*name)) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        let mut visited = BTreeSet::new();
        let mut cursor = Some(exit.clone());
        while let Some(name) = cursor {
            let Some(node) = index.get(&name) else {
                return Vec::new();
            };
            if !visited.insert(name.clone()) || result.len() >= 16 {
                return Vec::new();
            }
            result.push(*node);
            cursor = node
                .payload
                .get("dialer-proxy")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        result.reverse();
        result
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ConnectionSnapshot {
    #[serde(default, rename = "uploadTotal")]
    pub upload_total: u64,
    #[serde(default, rename = "downloadTotal")]
    pub download_total: u64,
    // mihomo 在空闲时返回 `"connections": null`，而 `serde(default)`
    // 只处理字段缺失，必须显式把 null 折叠成空列表。
    #[serde(default, deserialize_with = "null_as_default")]
    pub connections: Vec<CoreConnection>,
}

fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_port_validation_rejects_reserved_and_invalid() {
        assert!(validate_proxy_port(7890).is_ok());
        assert!(validate_proxy_port(8080).is_ok());
        assert!(validate_proxy_port(65535).is_ok());
        // 低于 1024 属于系统保留段。
        assert!(validate_proxy_port(80).is_err());
        // 内部保留端口：探测 / 聚合 / 控制 / Xray。
        assert!(validate_proxy_port(PROBE_PORT).is_err());
        assert!(validate_proxy_port(AGGREGATE_PORT).is_err());
        assert!(validate_proxy_port(CONTROLLER_PORT).is_err());
        assert!(validate_proxy_port(crate::logic::kernel::XRAY_SOCKS_PORT).is_err());
        assert!(validate_proxy_port(crate::logic::kernel::XRAY_API_PORT).is_err());
    }

    #[test]
    fn preferences_default_proxy_port_matches_runtime_default() {
        let preferences = Preferences::default();
        assert_eq!(preferences.proxy_port, DEFAULT_PROXY_PORT);
        // 旧版 preferences.json（无 proxyPort 字段）应回退到默认端口。
        let legacy: Preferences = serde_json::from_str(r#"{"sources":[]}"#).expect("legacy config");
        assert_eq!(legacy.proxy_port, DEFAULT_PROXY_PORT);
    }

    #[test]
    fn connection_snapshot_accepts_null_and_missing_connections() {
        let idle: ConnectionSnapshot = serde_json::from_str(
            r#"{"downloadTotal":1,"uploadTotal":2,"connections":null,"memory":0}"#,
        )
        .expect("空闲快照应能解析");
        assert!(idle.connections.is_empty());
        assert_eq!(idle.download_total, 1);
        assert_eq!(idle.upload_total, 2);

        let minimal: ConnectionSnapshot = serde_json::from_str("{}").expect("缺失字段应使用默认值");
        assert!(minimal.connections.is_empty());
    }
}
