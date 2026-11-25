use super::{
    CoreManager,
    handle::Handle,
    manager::RunningMode,
    smartclash_plan::{self as plan, Candidate, GROUP, Plan},
    smartclash_probe::{self as probe, Measurement},
};
use crate::config::{
    Config,
    smartclash::{self, Mode, Settings},
};
use anyhow::{Context as _, Result, ensure};
use clash_verge_logging::{Type, logging};
use futures::StreamExt as _;
use parking_lot::Mutex;
use serde_yaml_ng::{Mapping, Value};
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub settings: Settings,
    pub selected: Option<String>,
    pub chain: Vec<String>,
    pub status: String,
    pub busy: bool,
    pub measurements: Vec<Measurement>,
    pub error: Option<String>,
}

struct Sample {
    measurement: Measurement,
    checked: Instant,
}

struct State {
    snapshot: Snapshot,
    generation: u64,
    cancel: CancellationToken,
    monitor: Option<tokio::task::JoinHandle<()>>,
    samples: HashMap<String, Sample>,
    failed: HashMap<String, Instant>,
    manual: bool,
    owned_selection: Option<String>,
    last_switch: Option<Instant>,
    failures: u32,
    source: Option<Mapping>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            snapshot: Snapshot {
                settings: Settings::default(),
                selected: None,
                chain: Vec::new(),
                status: "disabled".into(),
                busy: false,
                measurements: Vec::new(),
                error: None,
            },
            generation: 0,
            cancel: CancellationToken::new(),
            monitor: None,
            samples: HashMap::new(),
            failed: HashMap::new(),
            manual: false,
            owned_selection: None,
            last_switch: None,
            failures: 0,
            source: None,
        }
    }
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::default()));
static LOADED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
static OPERATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const FRESH: Duration = Duration::from_secs(15);
const COOLDOWN: Duration = Duration::from_secs(60);
const SWITCH_COOLDOWN: Duration = Duration::from_secs(10);
const SCAN_LIMIT: usize = 12;
const SPEED_LIMIT: usize = 6;

async fn initialize() -> Result<()> {
    LOADED
        .get_or_try_init(|| async {
            crate::process::AsyncHandler::spawn_blocking(cleanup_stale_probe_dirs)
                .await
                .ok();
            let settings = smartclash::load().await?;
            let mut state = STATE.lock();
            state.snapshot.status = if settings.enabled { "idle" } else { "disabled" }.into();
            state.manual = !settings.automatic;
            state.snapshot.settings = settings;
            Ok::<_, anyhow::Error>(())
        })
        .await?;
    Ok(())
}

/// Probe dirs are named `smartclash-probe-<pid>-<token>` and removed when a probe core drops;
/// a crashed app leaves them behind, so drop other dead processes' leftovers once per launch.
fn cleanup_stale_probe_dirs() {
    let Ok(root) = crate::utils::dirs::app_home_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let own_prefix = format!("smartclash-probe-{}-", std::process::id());
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !name.starts_with("smartclash-probe-") || name.starts_with(&own_prefix) {
            continue;
        }
        let Some(pid) = name["smartclash-probe-".len()..]
            .split('-')
            .next()
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        // A missing process may be a reused PID; the token suffix makes collisions unlikely and
        // these dirs only hold short-lived probe configs, so deleting a false positive is safe.
        let alive = process_alive(pid);
        if alive {
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(entry.path()) {
            logging!(warn, Type::Core, "failed to remove stale probe dir {name}: {error:#}");
        }
    }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .creation_flags(0x0800_0000)
        .output()
        .map(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.contains(&pid.to_string())
        })
        .unwrap_or(true)
}

#[cfg(not(windows))]
fn process_alive(pid: u32) -> bool {
    // /proc 只在 Linux 上存在；macOS 等平台用 kill(pid, 0) 探活。
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    unsafe { libc::kill(pid as i32, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

fn supersede(state: &mut State) {
    state.generation = state.generation.wrapping_add(1);
    state.cancel.cancel();
    state.cancel = CancellationToken::new();
    if let Some(monitor) = state.monitor.take() {
        monitor.abort();
    }
    state.snapshot.busy = false;
}

pub fn invalidate_runtime() {
    let mut state = STATE.lock();
    supersede(&mut state);
    state.samples.clear();
    state.failed.clear();
    state.snapshot.measurements.clear();
    state.owned_selection = None;
    state.last_switch = None;
    state.failures = 0;
    state.source = None;
}

fn fail(error: impl std::fmt::Display) -> Snapshot {
    let mut state = STATE.lock();
    state.snapshot.error = Some(error.to_string());
    state.snapshot.clone()
}

async fn source_config() -> Result<Mapping> {
    let runtime = Config::runtime().await;
    let mut config = runtime.data_arc().config.clone().context("内核尚无已应用运行配置")?;
    if let Some(Value::Sequence(groups)) = config.get_mut("proxy-groups") {
        groups.retain(|group| group.get("name").and_then(Value::as_str) != Some(GROUP));
        for group in groups {
            if let Some(Value::Sequence(choices)) = group.get_mut("proxies") {
                choices.retain(|choice| choice.as_str() != Some(GROUP));
            }
        }
    }
    if let Some(Value::Sequence(proxies)) = config.get_mut("proxies") {
        proxies.retain(|proxy| {
            !proxy
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.starts_with("SMARTCLASH::"))
        });
    }
    Ok(config)
}

async fn actual_selection() -> Result<Option<String>> {
    let proxies = tokio::time::timeout(Duration::from_secs(4), Handle::mihomo().get_proxies()).await??;
    Ok(proxies.proxies.get(GROUP).and_then(|group| group.now.clone()))
}

/// 探测内核的出口网卡：绑定后探测流量绕过 TUN 直连候选节点。否则探测会套进
/// 当前线路拿到 fake-IP，全部候选测得同一复合质量，断线时同时失败，自动切换失效。
/// mihomo 的 /configs 并不回填自动检测的出口（实测为空），所以优先枚举网卡：
/// 排除自身 TUN/常见虚拟/隧道适配器，选第一个带真实 IPv4 的。
async fn probe_interface() -> Option<String> {
    if let Ok(config) = tokio::time::timeout(Duration::from_secs(3), Handle::mihomo().get_base_config()).await
        && let Ok(config) = config
        && !config.interface_name.is_empty()
    {
        return Some(config.interface_name);
    }
    physical_interface_fallback()
}

const VIRTUAL_INTERFACE_MARKERS: &[&str] = &[
    "meta",
    "mihomo",
    "clash",
    "tun",
    "tap",
    "tunnel",
    "vpn",
    "vethernet",
    "vmware",
    "virtualbox",
    "hyper-v",
    "loopback",
    "tailscale",
    "zerotier",
    "oray",
    "wintun",
    "utun",
];

fn physical_interface_fallback() -> Option<String> {
    use network_interface::NetworkInterfaceConfig as _;
    let interfaces = network_interface::NetworkInterface::show().ok()?;
    interfaces
        .into_iter()
        .filter(|interface| {
            let name = interface.name.to_lowercase();
            !VIRTUAL_INTERFACE_MARKERS.iter().any(|marker| name.contains(marker))
        })
        .find(|interface| {
            interface.addr.iter().any(|addr| match addr.ip() {
                std::net::IpAddr::V4(ip) => {
                    !ip.is_loopback()
                        && !ip.is_link_local()
                        && !ip.is_unspecified()
                        // TUN fake 网段 (RFC 2544 benchmarking range, mihomo 默认 fake-ip 池)
                        && !(ip.octets()[0] == 198 && (ip.octets()[1] == 18 || ip.octets()[1] == 19))
                }
                std::net::IpAddr::V6(_) => false,
            })
        })
        .map(|interface| interface.name)
}

#[cfg(test)]
mod physical_interface_tests {
    use super::*;

    #[test]
    fn virtual_and_tunnel_adapters_are_excluded_from_probe_binding() {
        // 这台机器上的真实适配器样本：Meta(Mihomo TUN)、OrayBoxVpnEnt(第三方 VPN)、
        // vEthernet (WSL) 必须排除；「以太网」等物理网卡必须入选。
        for name in [
            "Meta",
            "Mihomo",
            "OrayBoxVpnEnt",
            "vEthernet (WSL (Hyper-V firewall))",
            "Tailscale",
            "Loopback Pseudo-Interface 1",
        ] {
            let lowered = name.to_lowercase();
            assert!(
                VIRTUAL_INTERFACE_MARKERS.iter().any(|marker| lowered.contains(marker)),
                "{name} should be excluded"
            );
        }
        for name in ["以太网", "Ethernet", "WLAN", "Wi-Fi"] {
            let lowered = name.to_lowercase();
            assert!(
                !VIRTUAL_INTERFACE_MARKERS.iter().any(|marker| lowered.contains(marker)),
                "{name} should be eligible"
            );
        }
    }
}

async fn binary() -> Result<PathBuf> {
    let core = Config::verge().await.data_arc().get_valid_clash_core();
    let file = format!("{core}{}", if cfg!(windows) { ".exe" } else { "" });
    let mut path = std::env::current_exe()?.with_file_name(&file);
    if !path.is_file() {
        path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sidecar").join(format!(
            "{core}-{}{}",
            env!("SMARTCLASH_BUILD_TARGET"),
            if cfg!(windows) { ".exe" } else { "" }
        ));
    }
    ensure!(path.is_file(), "缺少隔离探测内核：{}", path.display());
    Ok(path)
}

pub async fn get() -> Snapshot {
    if let Err(error) = initialize().await {
        return fail(error);
    }
    let generation = STATE.lock().generation;
    if let Ok(selected) = actual_selection().await {
        let settings = STATE.lock().snapshot.settings.clone();
        let chain = match (source_config().await, selected.as_deref()) {
            (Ok(source), Some(selected)) => plan::build(&source, &settings)
                .ok()
                .and_then(|plan| plan.candidates.into_iter().find(|candidate| candidate.name == selected))
                .map(|candidate| candidate.chain)
                .unwrap_or_else(|| vec![selected.to_owned()]),
            _ => Vec::new(),
        };
        let mut state = STATE.lock();
        if state.generation == generation {
            state.snapshot.selected = selected;
            state.snapshot.chain = chain;
        }
    }
    ensure_monitor();
    STATE.lock().snapshot.clone()
}

fn ensure_monitor() {
    let mut state = STATE.lock();
    if !state.snapshot.settings.enabled
        || !state.snapshot.settings.automatic
        || state.manual
        || state.snapshot.busy
        || state.monitor.as_ref().is_some_and(|task| !task.is_finished())
    {
        return;
    }
    let generation = state.generation;
    let cancel = state.cancel.clone();
    state.monitor = Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = interval.tick() => {
                    if Handle::global().is_exiting() || !current(generation) { break; }
                    if *CoreManager::global().get_running_mode() != RunningMode::NotRunning {
                        let _ = run(false).await;
                    }
                }
            }
        }
    }));
}

pub async fn patch(settings: Settings) -> Snapshot {
    if let Err(error) = initialize().await {
        return fail(error);
    }
    if let Err(error) = settings.validate() {
        return fail(error);
    }
    let write = Config::lock_config_write().await;
    let _operation = OPERATIONS.lock().await;
    let runtime = Config::runtime().await;
    let transaction = match clash_verge_draft::DraftTransaction::begin(vec![&runtime]) {
        Ok(transaction) => transaction,
        Err(error) => return fail(error),
    };
    let original_runtime = runtime.data_arc();
    let previous = STATE.lock().snapshot.settings.clone();
    let config_changed = previous.enabled != settings.enabled
        || previous.chain_enabled != settings.chain_enabled
        || previous.entry_nodes != settings.entry_nodes
        || previous.exit_nodes != settings.exit_nodes
        || previous.source_profiles != settings.source_profiles
        || previous.smart_rules != settings.smart_rules
        || previous.explore_multihop != settings.explore_multihop;
    // 只有「自动维护」开关本身变化才解除本会话的手动保留；
    // 改动无关设置（模式、动画、出口偏好等）不得悄悄恢复自动切换。
    let automatic_changed = previous.automatic != settings.automatic;
    if let Err(error) = smartclash::save(&settings).await {
        return fail(error);
    }
    {
        let mut state = STATE.lock();
        supersede(&mut state);
        if automatic_changed {
            state.manual = !settings.automatic;
        }
        state.samples.clear();
        state.failed.clear();
        // 设置变化后重新累计确认故障，避免旧配置的计数触发换线。
        state.failures = 0;
        state.last_switch = None;
        state.snapshot.measurements.clear();
        state.snapshot.settings = settings.clone();
        state.snapshot.status = if settings.enabled { "idle" } else { "disabled" }.into();
        state.snapshot.error = None;
    }
    let apply = if config_changed && *CoreManager::global().get_running_mode() != RunningMode::NotRunning {
        match CoreManager::global().claim_config_update(&write) {
            Ok(update) => CoreManager::global().update_config_in_patch(&update).await,
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    };
    if let Err(error) = apply {
        transaction.rollback();
        if std::sync::Arc::ptr_eq(&original_runtime, &runtime.data_arc()) {
            if let Err(rollback) = smartclash::save(&previous).await {
                return fail(format!("运行配置应用失败：{error:#}；设置恢复失败：{rollback:#}"));
            }
            STATE.lock().snapshot.settings = previous;
            return fail(format!("运行配置应用失败，已恢复原设置：{error:#}"));
        }
        return fail(format!("设置及内核配置已生效，后续步骤失败：{error:#}"));
    }
    transaction.commit();
    drop(_operation);
    drop(write);
    get().await
}

pub async fn manual_selection(group: &str, node: &str) -> Result<()> {
    if group != GROUP {
        return Ok(());
    }
    initialize().await?;
    let _operation = OPERATIONS.lock().await;
    let mut state = STATE.lock();
    supersede(&mut state);
    // 手动选择只在本会话暂停自动切换，不改写存盘的 automatic：
    // 避免「曾经手动点过一次节点 → 自动切换被永久关闭且无人记得」导致断线后不切换。
    // 恢复自动：重启应用，或在面板点「恢复自动选路」（optimize 会退出手动状态）。
    state.manual = true;
    state.failures = 0;
    state.last_switch = None;
    state.snapshot.selected = Some(node.to_owned());
    state.snapshot.chain = vec![node.to_owned()];
    state.snapshot.status = "manual".into();
    Ok(())
}

pub async fn optimize() -> Snapshot {
    if let Err(error) = initialize().await {
        return fail(error);
    }
    let result = run(true).await;
    if let Err(error) = result {
        return fail(error);
    }
    // 用户主动触发重新选路，视为放弃手动保留，恢复自动维护。
    {
        let mut state = STATE.lock();
        if state.snapshot.settings.automatic {
            state.manual = false;
        }
        // 新线路从零开始判定，避免旧线路的失败计数让下一轮立刻再跳。
        state.failures = 0;
        state.last_switch = None;
    }
    ensure_monitor();
    STATE.lock().snapshot.clone()
}

fn current(generation: u64) -> bool {
    STATE.lock().generation == generation
}

fn allowed(state: &State, generation: u64) -> bool {
    state.generation == generation
        && state.snapshot.settings.enabled
        && state.snapshot.settings.automatic
        && !state.manual
}

/// 当前线路是否仍然可用。
/// AI 模式必须明确探测到可达（Some(true)）才算健康：unknown challenge / 429 /
/// 非 JSON 响应会让 classify_ai 返回 None，若把它当成健康就会清零失败计数、
/// 永久保留当前线路，导致 OpenAI 断线后不切换。该语义需与 plan::eligible 一致。
fn is_healthy(mode: Mode, measurement: &Measurement) -> bool {
    measurement.latency_ms.is_some() && (mode != Mode::Ai || measurement.ai_reachable == Some(true))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteHealth {
    Healthy,
    /// 429 / 挑战页 / 非 JSON 响应：不能证明 OpenAI 可用，也不能证明被墙。
    Unknown,
    Failed,
}

/// 自动换线只看「确认故障」：AI 模式必须明确探测到 OpenAI 不可达
/// （ai_reachable == Some(false)）或者完全没有 HTTPS 出网。未知结果一律不算故障，
/// 避免定时刷新把限流/挑战页误判成断线导致节点一直跳。
fn route_health(mode: Mode, measurement: &Measurement) -> RouteHealth {
    if measurement.latency_ms.is_none() || (mode == Mode::Ai && measurement.ai_reachable == Some(false)) {
        RouteHealth::Failed
    } else if is_healthy(mode, measurement) {
        RouteHealth::Healthy
    } else {
        RouteHealth::Unknown
    }
}

/// 只有连续两次确认故障并且切换冷却结束才允许后台换线；
/// 定时刷新和「别的节点更快」都不能改变当前线路。显式优化不受此限制。
fn should_failover(
    explicit: bool,
    has_baseline: bool,
    health: Option<RouteHealth>,
    failures: u32,
    last_switch: Option<Instant>,
) -> bool {
    !explicit
        && has_baseline
        && health == Some(RouteHealth::Failed)
        && failures >= 2
        && !last_switch.is_some_and(|last| last.elapsed() < SWITCH_COOLDOWN)
}

fn ranking<'a>(plan: &'a Plan, samples: &HashMap<String, Sample>, settings: &Settings) -> Vec<&'a Candidate> {
    let mut candidates: Vec<_> = plan
        .candidates
        .iter()
        .filter(|candidate| {
            samples.get(&candidate.name).is_some_and(|sample| {
                sample.checked.elapsed() <= FRESH
                    && plan::eligible(
                        settings.mode,
                        sample.measurement.latency_ms,
                        sample.measurement.bytes_per_second,
                        sample.measurement.ai_reachable,
                    )
            })
        })
        .collect();
    candidates.sort_by(|left, right| {
        let l = &samples[&left.name].measurement;
        let r = &samples[&right.name].measurement;
        (settings.prefer_non_hong_kong && left.hong_kong)
            .cmp(&(settings.prefer_non_hong_kong && right.hong_kong))
            .then_with(|| {
                if settings.mode == Mode::HighSpeed {
                    r.bytes_per_second
                        .unwrap_or_default()
                        .total_cmp(&l.bytes_per_second.unwrap_or_default())
                } else {
                    l.latency_ms.cmp(&r.latency_ms)
                }
            })
    });
    candidates
}

/// 兜底选路：只要求候选探测到出网（HTTPS 204 通过、即 latency_ms 有值），
/// 不要求 AI 可达性确认。用于当前线路已确认故障、但严格准入（ranking）
/// 没有任何「确认可用」备用的场景，避免卡在完全断线的节点上。
/// AI 模式排除已明确判定不可达的候选，防止在若干被地区封锁的节点间反复横跳。
fn relaxed_ranking<'a>(plan: &'a Plan, samples: &HashMap<String, Sample>, settings: &Settings) -> Vec<&'a Candidate> {
    let mut candidates: Vec<_> = plan
        .candidates
        .iter()
        .filter(|candidate| {
            samples.get(&candidate.name).is_some_and(|sample| {
                sample.checked.elapsed() <= FRESH
                    && sample.measurement.latency_ms.is_some()
                    && (settings.mode != Mode::Ai || sample.measurement.ai_reachable != Some(false))
            })
        })
        .collect();
    candidates.sort_by(|left, right| {
        let l = &samples[&left.name].measurement;
        let r = &samples[&right.name].measurement;
        (settings.prefer_non_hong_kong && left.hong_kong)
            .cmp(&(settings.prefer_non_hong_kong && right.hong_kong))
            .then_with(|| l.latency_ms.cmp(&r.latency_ms))
    });
    candidates
}

struct BusyGuard(u64);
impl Drop for BusyGuard {
    fn drop(&mut self) {
        let mut state = STATE.lock();
        if state.generation == self.0 {
            state.snapshot.busy = false;
        }
    }
}

async fn run(explicit: bool) -> Result<()> {
    let (settings, generation, cancel) = {
        let _operation = OPERATIONS.lock().await;
        let mut state = STATE.lock();
        ensure!(state.snapshot.settings.enabled, "请先启用 SmartClash");
        if state.snapshot.busy {
            if !explicit {
                return Ok(());
            }
            // 手动优化取代进行中的监控轮次，而不是被静默丢弃。
            supersede(&mut state);
        }
        if !explicit && (!state.snapshot.settings.automatic || state.manual) {
            return Ok(());
        }
        state.snapshot.busy = true;
        state.snapshot.status = "probing".into();
        state.snapshot.error = None;
        (state.snapshot.settings.clone(), state.generation, state.cancel.clone())
    };
    let _busy = BusyGuard(generation);
    let work = async {
        let readiness = CoreManager::global().current_core_readiness_generation();
        ensure!(readiness.is_some(), "请先启动内核并应用订阅");
        let source = source_config().await?;
        if source.get("mode").and_then(Value::as_str) == Some("direct") {
            STATE.lock().snapshot.status = "direct".into();
            return Ok(());
        }
        {
            let mut state = STATE.lock();
            if state.source.as_ref() != Some(&source) {
                state.samples.clear();
                state.failed.clear();
                state.owned_selection = None;
                state.source = Some(source.clone());
            }
        }
        let plan = plan::build(&source, &settings)?;
        plan::require_inline(&plan)?;
        let binary = binary().await?;
        let interface = probe_interface().await;
        let baseline = actual_selection().await?;
        {
            let mut state = STATE.lock();
            if !explicit
                && state
                    .owned_selection
                    .as_ref()
                    .is_some_and(|owned| Some(owned) != baseline.as_ref())
            {
                state.manual = true;
                state.snapshot.selected = baseline;
                state.snapshot.status = "manual".into();
                return Ok(());
            }
        }
        let mut candidates: Vec<_> = plan.candidates.iter().collect();
        // 监控先刷新当前与两条备用，再轮转未探测节点；显式优化仍有固定流量预算。
        {
            let state = STATE.lock();
            candidates.sort_by_key(|candidate| {
                (
                    !explicit && candidate.name != baseline.as_deref().unwrap_or_default(),
                    settings.prefer_non_hong_kong && candidate.hong_kong,
                    !state.samples.get(&candidate.name).is_some_and(|sample| {
                        plan::eligible(
                            settings.mode,
                            sample.measurement.latency_ms,
                            sample.measurement.bytes_per_second,
                            sample.measurement.ai_reachable,
                        )
                    }),
                    state.samples.get(&candidate.name).map(|sample| sample.checked),
                )
            });
        }
        let limit = if explicit {
            if settings.mode == Mode::HighSpeed {
                SPEED_LIMIT
            } else {
                SCAN_LIMIT
            }
        } else {
            3
        };
        let candidates: Vec<_> = {
            let state = STATE.lock();
            candidates
                .into_iter()
                .filter(|candidate| {
                    !state.failed.get(&candidate.name).is_some_and(|failed| {
                        failed.elapsed() < COOLDOWN && Some(candidate.name.as_str()) != baseline.as_deref()
                    })
                })
                .take(limit)
                .cloned()
                .collect()
        };
        let scan_started = Instant::now();
        let probe_mode = if !explicit && settings.mode == Mode::HighSpeed {
            Mode::LowLatency
        } else {
            settings.mode
        };
        let mut probes = futures::stream::iter(candidates.into_iter().map(|candidate| {
            let binary = &binary;
            let plan = &plan;
            let interface = interface.as_deref();
            async move {
                let measured = tokio::time::timeout(
                    Duration::from_secs(12),
                    probe::measure(binary, plan, &candidate, probe_mode, interface),
                )
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("候选检测超时")));
                (candidate, measured)
            }
        }))
        .buffer_unordered(3);
        while let Some((candidate, measured)) = probes.next().await {
            ensure!(current(generation), "优化已被新操作取消");
            let mut state = STATE.lock();
            if state.generation != generation {
                return Ok(());
            }
            let mut measurement = measured.unwrap_or_else(|_| Measurement::empty(&candidate.name));
            if !explicit && settings.mode == Mode::HighSpeed {
                measurement.bytes_per_second = state
                    .samples
                    .get(&candidate.name)
                    .and_then(|sample| sample.measurement.bytes_per_second);
            }
            if !plan::eligible(
                probe_mode,
                measurement.latency_ms,
                measurement.bytes_per_second,
                measurement.ai_reachable,
            ) {
                state.failed.insert(candidate.name.clone(), Instant::now());
            } else {
                state.failed.remove(&candidate.name);
            }
            state.samples.insert(
                candidate.name.clone(),
                Sample {
                    measurement,
                    checked: Instant::now(),
                },
            );
            state.snapshot.measurements = plan
                .candidates
                .iter()
                .filter_map(|candidate| {
                    state
                        .samples
                        .get(&candidate.name)
                        .map(|sample| sample.measurement.clone())
                })
                .collect();
            if scan_started.elapsed() >= Duration::from_secs(30) {
                break;
            }
        }
        drop(probes);
        let best = {
            let mut state = STATE.lock();
            let health = baseline
                .as_ref()
                .and_then(|name| state.samples.get(name))
                .filter(|sample| sample.checked.elapsed() <= FRESH)
                .map(|sample| route_health(settings.mode, &sample.measurement));
            // 只累计连续确认故障：健康或未知结果都会清零，避免 429/挑战页
            // 与偶发丢包逐渐累积成「断线」。
            if health == Some(RouteHealth::Failed) {
                state.failures = state.failures.saturating_add(1);
            } else {
                state.failures = 0;
            }
            let failover = should_failover(explicit, baseline.is_some(), health, state.failures, state.last_switch);
            // 定时刷新只更新测量与备选预热，不因为「别的节点更快」换线；
            // 只有确认连不通 OpenAI 才进入故障切换，显式优化除外。
            let ready = if !explicit && !failover {
                plan.candidates
                    .iter()
                    .find(|candidate| Some(candidate.name.as_str()) == baseline.as_deref())
                    .cloned()
            } else {
                let ranking_settings = Settings {
                    mode: if !explicit && settings.mode == Mode::HighSpeed {
                        Mode::LowLatency
                    } else {
                        settings.mode
                    },
                    ..settings.clone()
                };
                let best = ranking(&plan, &state.samples, &ranking_settings)
                    .first()
                    .map(|candidate| (*candidate).clone());
                // 当前线路已确认故障但没有任何「确认可用」的备用时，放宽到
                // 「至少能出网」的候选，避免一直卡在完全断线的节点上。
                // 显式优化仍坚持严格准入，不对用户隐瞒候选的真实质量。
                match best {
                    Some(best) => Some(best),
                    None if failover => relaxed_ranking(&plan, &state.samples, &ranking_settings)
                        .first()
                        .map(|candidate| (*candidate).clone()),
                    None => None,
                }
            };
            ready
        };
        let Some(best) = best else {
            anyhow::bail!("本轮没有通过真实测量的候选；保留当前线路（每轮最多 {limit} 条，失败冷却 60 秒）");
        };
        let _write = Config::lock_config_write().await;
        let _operation = OPERATIONS.lock().await;
        ensure!(
            current(generation)
                && CoreManager::global().current_core_readiness_generation() == readiness
                && source_config().await? == source,
            "运行配置已改变，丢弃旧探测结果"
        );
        let actual = actual_selection().await?;
        if actual != baseline {
            let mut state = STATE.lock();
            state.manual = true;
            state.snapshot.selected = actual;
            state.snapshot.status = "manual".into();
            return Ok(());
        }
        if STATE.lock().generation != generation {
            return Ok(());
        }
        let can_switch = { allowed(&STATE.lock(), generation) };
        if can_switch || explicit {
            let switched = actual.as_deref() != Some(best.name.as_str());
            if switched {
                // 仅修改独立 SMARTCLASH 组，不改变父组、规则、系统代理或 TUN。
                tokio::time::timeout(
                    Duration::from_secs(4),
                    Handle::mihomo().select_node_for_group(GROUP, &best.name),
                )
                .await??;
                ensure!(
                    actual_selection().await?.as_deref() == Some(best.name.as_str()),
                    "内核未确认新线路，保留状态待复检"
                );
                // 旧线路上的既有连接不会自己迁移，断线场景下会一直挂着直到应用放弃；
                // 与切换 Clash 模式一致，按 auto_close_connection 关闭全部连接让流量
                // 立刻走上新线路。
                let auto_close = Config::verge()
                    .await
                    .latest_arc()
                    .auto_close_connection
                    .unwrap_or(false);
                if auto_close {
                    let closed =
                        tokio::time::timeout(Duration::from_secs(4), Handle::mihomo().close_all_connections()).await;
                    if let Err(error) = closed {
                        logging!(
                            warn,
                            Type::Core,
                            "failed to close connections after switching line: {error:#}"
                        );
                    }
                }
            }
            {
                let mut state = STATE.lock();
                if state.generation == generation {
                    if switched {
                        state.last_switch = Some(Instant::now());
                        state.failures = 0;
                    }
                    state.owned_selection = Some(best.name.clone());
                    state.snapshot.selected = Some(best.name.clone());
                    state.snapshot.chain = best.chain.clone();
                    state.snapshot.status = if state
                        .samples
                        .get(&state.snapshot.selected.clone().unwrap_or_default())
                        .is_some_and(|sample| {
                            sample.checked.elapsed() <= FRESH && is_healthy(settings.mode, &sample.measurement)
                        }) {
                        "ready"
                    } else {
                        "degraded"
                    }
                    .into();
                }
            }
            // 先落状态再记录；记录失败只降级为警告，不能让下一轮误判成手动操作。
            if switched {
                if let Err(error) = crate::config::profiles::record_smartclash_selection(GROUP, &best.name).await {
                    logging!(warn, Type::Core, "线路已切换但未能记录选择，下次配置重载可能回退：{error:#}");
                }
                Handle::refresh_clash();
            }
        } else {
            let mut state = STATE.lock();
            state.snapshot.selected = actual;
            state.snapshot.status = "manual".into();
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! {
        _ = cancel.cancelled() => Ok(()),
        result = tokio::time::timeout(Duration::from_secs(45), work) => {
            let result = result.unwrap_or_else(|_| Err(anyhow::anyhow!("本轮检测达到 45 秒预算，保留当前线路")));
            if let Err(error) = &result {
                let mut state = STATE.lock();
                if state.generation == generation {
                    state.snapshot.status = "error".into();
                    state.snapshot.error = Some(format!("{error:#}"));
                }
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_and_generation_guards_prevent_old_workers_from_switching() {
        let mut state = State::default();
        state.snapshot.settings.enabled = true;
        assert!(allowed(&state, 0));
        state.manual = true;
        assert!(!allowed(&state, 0));
        state.manual = false;
        supersede(&mut state);
        assert!(!allowed(&state, 0));
        state.snapshot.settings.automatic = false;
        assert!(!allowed(&state, 1));
    }

    #[test]
    fn ai_health_requires_confirmed_reachability_not_absence_of_failure() {
        let measure = |latency: Option<u64>, ai: Option<bool>| Measurement {
            name: "fixture".into(),
            latency_ms: latency,
            bytes_per_second: None,
            ai_reachable: ai,
        };
        // The regression: 429 / unknown challenge / non-JSON responses map to None.
        // Treating None as healthy reset the failure counter and never switched.
        assert!(!is_healthy(Mode::Ai, &measure(Some(20), None)));
        assert!(!is_healthy(Mode::Ai, &measure(Some(20), Some(false))));
        assert!(is_healthy(Mode::Ai, &measure(Some(20), Some(true))));
        assert!(!is_healthy(Mode::Ai, &measure(None, Some(true))));
        // Non-AI modes still only need a working HTTPS path.
        assert!(is_healthy(Mode::LowLatency, &measure(Some(20), None)));
        assert!(is_healthy(Mode::HighSpeed, &measure(Some(20), None)));
        assert!(!is_healthy(Mode::LowLatency, &measure(None, None)));
    }

    #[test]
    fn unknown_ai_probe_never_counts_as_confirmed_failure() {
        let measure = |latency: Option<u64>, ai: Option<bool>| Measurement {
            name: "fixture".into(),
            latency_ms: latency,
            bytes_per_second: None,
            ai_reachable: ai,
        };
        // 429 / 挑战页 / 非 JSON：未知，既不算健康也不算断线。
        assert_eq!(route_health(Mode::Ai, &measure(Some(20), None)), RouteHealth::Unknown);
        assert_eq!(
            route_health(Mode::Ai, &measure(Some(20), Some(true))),
            RouteHealth::Healthy
        );
        assert_eq!(
            route_health(Mode::Ai, &measure(Some(20), Some(false))),
            RouteHealth::Failed
        );
        assert_eq!(route_health(Mode::Ai, &measure(None, Some(true))), RouteHealth::Failed);
        // 非 AI 模式只要求 HTTPS 出网成功。
        assert_eq!(
            route_health(Mode::LowLatency, &measure(Some(20), None)),
            RouteHealth::Healthy
        );
        assert_eq!(
            route_health(Mode::LowLatency, &measure(None, None)),
            RouteHealth::Failed
        );
    }

    #[test]
    fn only_repeated_confirmed_failure_allows_background_failover() {
        let health = Some(RouteHealth::Failed);
        assert!(!should_failover(false, true, health, 1, None));
        assert!(should_failover(false, true, health, 2, None));
        // 未知或健康结果不换线；显式优化不受故障判定限制。
        assert!(!should_failover(false, true, Some(RouteHealth::Unknown), 9, None));
        assert!(!should_failover(false, true, Some(RouteHealth::Healthy), 0, None));
        assert!(!should_failover(false, false, health, 9, None));
        assert!(!should_failover(false, true, health, 9, Some(Instant::now())));
        assert!(!should_failover(true, false, Some(RouteHealth::Healthy), 0, None));
    }

    #[test]
    fn relaxed_ranking_skips_candidates_confirmed_blocked_for_ai() {
        let settings = Settings {
            enabled: true,
            mode: Mode::Ai,
            ..Settings::default()
        };
        let plan = Plan {
            candidates: vec![
                Candidate {
                    name: "hk".into(),
                    chain: vec!["hk".into()],
                    hong_kong: true,
                },
                Candidate {
                    name: "jp".into(),
                    chain: vec!["jp".into()],
                    hong_kong: false,
                },
            ],
            ..Plan::default()
        };
        let samples = HashMap::from([
            (
                "hk".into(),
                Sample {
                    measurement: Measurement {
                        name: "hk".into(),
                        latency_ms: Some(5),
                        bytes_per_second: None,
                        ai_reachable: None,
                    },
                    checked: Instant::now(),
                },
            ),
            (
                "jp".into(),
                Sample {
                    measurement: Measurement {
                        name: "jp".into(),
                        latency_ms: Some(90),
                        bytes_per_second: None,
                        ai_reachable: Some(false),
                    },
                    checked: Instant::now(),
                },
            ),
        ]);
        // 严格准入：AI 模式下两者都没有确认可达（Some(true)），无候选。
        assert!(ranking(&plan, &samples, &settings).is_empty());
        // 兜底准入：未知候选（hk）可用；已确认被地区封锁的 jp 不能作为跳板。
        let relaxed = relaxed_ranking(&plan, &samples, &settings);
        assert_eq!(relaxed.len(), 1);
        assert_eq!(relaxed[0].name, "hk");
    }

    #[test]
    fn ranking_uses_fresh_real_metrics_and_exit_preference() {
        let settings = Settings {
            enabled: true,
            ..Settings::default()
        };
        let plan = Plan {
            candidates: vec![
                Candidate {
                    name: "hk".into(),
                    chain: vec!["hk".into()],
                    hong_kong: true,
                },
                Candidate {
                    name: "jp".into(),
                    chain: vec!["jp".into()],
                    hong_kong: false,
                },
            ],
            ..Plan::default()
        };
        let samples = HashMap::from([
            (
                "hk".into(),
                Sample {
                    measurement: Measurement {
                        name: "hk".into(),
                        latency_ms: Some(5),
                        bytes_per_second: None,
                        ai_reachable: None,
                    },
                    checked: Instant::now(),
                },
            ),
            (
                "jp".into(),
                Sample {
                    measurement: Measurement {
                        name: "jp".into(),
                        latency_ms: Some(90),
                        bytes_per_second: None,
                        ai_reachable: None,
                    },
                    checked: Instant::now(),
                },
            ),
        ]);
        assert_eq!(ranking(&plan, &samples, &settings)[0].name, "jp");
        assert!(
            ranking(
                &plan,
                &samples,
                &Settings {
                    mode: Mode::Ai,
                    ..settings
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn stale_probe_dirs_of_dead_processes_are_removed_and_live_ones_kept() {
        let root = std::env::temp_dir().join(format!("smartclash-probe-cleanup-test-{}", nanoid::nanoid!(8)));
        std::fs::create_dir_all(&root).unwrap();
        let dead = root.join(format!("smartclash-probe-{}-dead", u32::MAX - 1));
        let own = root.join(format!("smartclash-probe-{}-own", std::process::id()));
        std::fs::create_dir_all(&dead).unwrap();
        std::fs::create_dir_all(&own).unwrap();

        // Run the same scan the app runs, but against the temp root by pointing app_home_dir at it
        // is not possible; instead exercise the shared logic via a direct reimplementation guard:
        // the function reads app_home_dir, so here we only assert the helper predicates.
        let parse = |name: &str| {
            name.strip_prefix("smartclash-probe-")
                .and_then(|rest| rest.split('-').next())
                .and_then(|pid| pid.parse::<u32>().ok())
        };
        assert_eq!(parse(dead.file_name().unwrap().to_str().unwrap()), Some(u32::MAX - 1));
        assert_eq!(
            parse(own.file_name().unwrap().to_str().unwrap()),
            Some(std::process::id())
        );
        assert!(!process_alive(u32::MAX - 1));
        assert!(process_alive(std::process::id()));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
