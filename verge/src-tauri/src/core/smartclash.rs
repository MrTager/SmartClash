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
use futures::StreamExt as _;
use parking_lot::Mutex;
use serde_yaml_ng::{Mapping, Value};
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
const SCAN_LIMIT: usize = 12;
const SPEED_LIMIT: usize = 6;

async fn initialize() -> Result<()> {
    LOADED
        .get_or_try_init(|| async {
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
    if let Err(error) = smartclash::save(&settings).await {
        return fail(error);
    }
    {
        let mut state = STATE.lock();
        supersede(&mut state);
        state.manual = !settings.automatic;
        state.samples.clear();
        state.failed.clear();
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
    measurement.latency_ms.is_some()
        && (mode != Mode::Ai || measurement.ai_reachable == Some(true))
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
fn relaxed_ranking<'a>(
    plan: &'a Plan,
    samples: &HashMap<String, Sample>,
    settings: &Settings,
) -> Vec<&'a Candidate> {
    let mut candidates: Vec<_> = plan
        .candidates
        .iter()
        .filter(|candidate| {
            samples.get(&candidate.name).is_some_and(|sample| {
                sample.checked.elapsed() <= FRESH && sample.measurement.latency_ms.is_some()
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
            return Ok(());
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
            async move {
                let measured = tokio::time::timeout(
                    Duration::from_secs(12),
                    probe::measure(binary, plan, &candidate, probe_mode),
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
            let healthy = baseline
                .as_ref()
                .and_then(|name| state.samples.get(name))
                .filter(|sample| sample.checked.elapsed() <= FRESH)
                .map(|sample| is_healthy(settings.mode, &sample.measurement));
            if healthy == Some(true) {
                state.failures = 0;
            } else if healthy == Some(false) {
                state.failures = state.failures.saturating_add(1);
            }
            let threshold = if settings.mode == Mode::Ai { 1 } else { 2 };
            let preserve = !explicit
                && baseline.is_some()
                && (healthy != Some(false)
                    || state.failures < threshold
                    || state
                        .last_switch
                        .is_some_and(|last| last.elapsed() < Duration::from_secs(30)));
            let ready = if preserve {
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
                    None if !explicit && healthy == Some(false) => {
                        relaxed_ranking(&plan, &state.samples, &ranking_settings)
                            .first()
                            .map(|candidate| (*candidate).clone())
                    }
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
            if actual.as_deref() != Some(&best.name) {
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
                crate::config::profiles::record_smartclash_selection(GROUP, &best.name).await?;
                Handle::refresh_clash();
            }
            let mut state = STATE.lock();
            if state.generation == generation {
                if actual.as_deref() != Some(best.name.as_str()) {
                    state.last_switch = Some(Instant::now());
                    state.failures = 0;
                }
                state.owned_selection = Some(best.name.clone());
                state.snapshot.selected = Some(best.name);
                state.snapshot.chain = best.chain;
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
    fn relaxed_ranking_accepts_reachable_candidates_when_no_confirmed_backup() {
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
        // 兜底准入：只看能否出网，AI 可达性不作要求，仍优先非香港出口。
        let relaxed = relaxed_ranking(&plan, &samples, &settings);
        assert_eq!(relaxed.len(), 2);
        assert_eq!(relaxed[0].name, "jp");
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
}
