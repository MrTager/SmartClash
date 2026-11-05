use crate::logic::configuration;
use crate::logic::dynamic_router::{DynamicRouter, RoutePlan};
use crate::logic::health::{now_ms, HealthPolicy};
use crate::logic::kernel::{KernelKind, RulesSupport};
use crate::logic::model::{
    LinkMeasurements, NodeLatency, ProbeResult, ProxyNode, RoutingMode, TrafficMode, PROBE_PORT,
    proxy_port,
};
use crate::logic::ranking::{ModeRanking, NodeRanking};
use crate::services::core_client::CoreClient;
use crate::services::core_process;
use crate::services::core_probe;
use crate::services::network;
use crate::services::paths;
use crate::services::subscription_service;
use crate::services::system_proxy::SystemProxy;
use crate::state::activity;
use crate::state::traffic;
use crate::state::tun_transition::{self, TunRuntime};
use crate::state::{with_store, Shared};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// 当前内核类型。
fn kernel_of(shared: &Shared) -> KernelKind {
    with_store(shared, |store| store.preferences.kernel)
}

/// 构建指向当前内核控制接口的客户端。
fn client_of(shared: &Shared) -> CoreClient {
    let (kind, secret) = with_store(shared, |store| {
        (store.preferences.kernel, store.preferences.secret.clone())
    });
    CoreClient::new(kind, &secret)
}

/// 过滤出当前内核支持的节点，保持原顺序并去重。
fn first_supported_name(kind: KernelKind, nodes: &[ProxyNode], preferred: Option<&str>) -> String {
    if let Some(name) = preferred {
        if nodes.iter().any(|node| node.name == name && kind.supports_node(node)) {
            return name.to_string();
        }
    }
    nodes
        .iter()
        .find(|node| kind.supports_node(node))
        .map(|node| node.name.clone())
        .unwrap_or_default()
}

/// 重启常驻内核，并恢复重启前的动态路由、TUN 与系统代理状态。
fn restart_core(
    shared: &Shared,
    preferred: Option<String>,
    resume_system_proxy: bool,
) -> Result<(), String> {
    let (running, tun, dynamic, plan, selected, system_proxy, epoch) = with_store(shared, |store| {
        (
            store.running,
            store.tun,
            store.dynamic_mode,
            store.route_plan.clone(),
            store.selected.clone(),
            store.system_proxy,
            store.epoch,
        )
    });
    if !running {
        return Ok(());
    }
    do_stop(shared)?;
    with_store(shared, |store| {
        store.dynamic_mode = dynamic;
        store.route_plan = plan;
    });
    let preferred = preferred.or_else(|| {
        if selected.is_empty() {
            None
        } else {
            Some(selected)
        }
    });
    do_start(shared, tun, preferred, Purpose::Connection)?;
    if tun {
        verify_tun(shared, epoch)?;
    }
    if resume_system_proxy && system_proxy {
        enable_system_proxy(shared, epoch)?;
    }
    start_monitor(shared);
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Connection,
    LatencyTest,
}

/// AI 模式连接前的 OpenAI 预检上限：避免一键连接被地区限制节点拖死。
const AI_VERIFY_LIMIT: usize = 8;
const AI_VERIFY_BUDGET_MS: u64 = 25_000;

fn fail(shared: &Shared, message: impl Into<String>) {
    let message = message.into();
    with_store(shared, |store| {
        store.error = Some(message.clone());
        store.log(&message);
    });
}

fn log(shared: &Shared, message: &str) {
    with_store(shared, |store| store.log(message));
}

/// 用户按下“停止”后 epoch 自增；长流程据此尽快退出，避免停止后又把内核拉起来。
fn cancelled(shared: &Shared, epoch: u64) -> bool {
    with_store(shared, |store| store.stopping || store.epoch != epoch)
}

pub fn perform<F>(shared: &Shared, ctx: &egui::Context, action: F)
where
    F: FnOnce(&Shared, &egui::Context) + Send + 'static,
{
    {
        let mut store = shared.lock().unwrap();
        if store.busy || store.stopping {
            return;
        }
        if store.is_testing_latency {
            store.error = Some("请先取消延迟测试，再执行连接或配置操作".into());
            return;
        }
        store.busy = true;
    }
    let shared = Arc::clone(shared);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        action(&shared, &ctx);
        with_store(&shared, |store| store.busy = false);
        ctx.request_repaint();
    });
}

// ---------------------------------------------------------------- 订阅

pub fn refresh(shared: &Shared, ctx: &egui::Context) {
    perform(shared, ctx, |store, _ctx| {
        if let Err(error) = do_refresh(store) {
            fail(store, error);
        }
    });
}

fn do_refresh(shared: &Shared) -> Result<(), String> {
    let (sources, running, secret, kind) = with_store(shared, |store| {
        (
            store.preferences.sources.clone(),
            store.running,
            store.preferences.secret.clone(),
            store.preferences.kernel,
        )
    });
    if with_store(shared, |s| s.is_testing_latency) {
        return Err("请先取消延迟测试，再更新订阅".into());
    }
    if with_store(shared, |s| s.dynamic_mode) {
        return Err("智能路由运行中，请停止后更新订阅".into());
    }
    if with_store(shared, |s| s.tun) {
        return Err("更新订阅前请先关闭 TUN 并停止内核".into());
    }

    let update = subscription_service::update(&sources, running)?;

    let mut refreshed: Option<(Vec<ProxyNode>, Vec<ProxyNode>, String)> = None;
    let mut restart_preferred: Option<String> = None;
    if running {
        let (plan, selected, traffic_mode) = with_store(shared, |s| {
            (s.route_plan.clone(), s.selected.clone(), s.preferences.traffic_mode)
        });
        // 订阅更新后旧计划里的节点可能已被删除；必须过滤，否则生成的
        // SMART 组会引用不存在的代理，内核校验直接失败。
        let available: Vec<ProxyNode> = if plan.candidates.is_empty() {
            update.nodes.clone()
        } else {
            let kept: Vec<ProxyNode> = plan
                .candidates
                .iter()
                .filter(|old| update.nodes.iter().any(|new| new.id == old.id))
                .cloned()
                .collect();
            if kept.is_empty() {
                update.nodes.clone()
            } else {
                kept
            }
        };
        let next = if traffic_mode.uses_proxy() {
            available
                .iter()
                .find(|n| n.name == selected)
                .or_else(|| available.first())
                .map(|n| n.name.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };
        if kind == KernelKind::Mihomo {
            let built = configuration::make_for(
                kind,
                &update.nodes,
                &secret,
                false,
                Some(&available),
                &plan.definitions,
                None,
                Some(&selected),
                traffic_mode,
            )?;
            for warning in &built.warnings {
                log(shared, warning);
            }
            let temp = paths::data_root().join("reload.json");
            paths::write_private(&temp, &built.data)?;
            core_process::validate(kind, &built.data)?;
            let client = CoreClient::new(kind, &secret);
            client.reload_config_path(&temp.display().to_string())?;
            if !next.is_empty() {
                client.select(&next, "SMART")?;
            }
        } else {
            // sing-box / Xray 不支持可靠的热加载；更新订阅后整体重启内核。
            restart_preferred = Some(next.clone());
        }
        refreshed = Some((available, plan.definitions, next));
    }

    with_store(shared, |store| {
        store.nodes = update.nodes.clone();
        if let Some((candidates, definitions, selected)) = refreshed {
            store.route_plan = RoutePlan {
                candidates,
                definitions,
            };
            store.selected = selected;
            store.publish_route();
        }
        let _ = store.aggregate.update(&store.nodes.clone());
        store.last_update = now_ms();
        store.log(&format!("订阅已合并：{} 个去重节点", store.nodes.len()));
        for warning in &update.warnings {
            store.log(warning);
        }
    });
    if let Some(preferred) = restart_preferred {
        let preferred = if preferred.is_empty() {
            None
        } else {
            Some(preferred)
        };
        restart_core(shared, preferred, true)?;
    }
    Ok(())
}

pub fn add_source(shared: &Shared, name: &str, url: &str) -> Result<(), String> {
    let url = url.trim().to_string();
    if !url.starts_with("https://")
        || url
            .trim_start_matches("https://")
            .split('/')
            .next()
            .map(|host| host.is_empty() || host.contains('@'))
            .unwrap_or(true)
    {
        return Err("请输入 HTTPS Clash 订阅链接".into());
    }
    with_store(shared, |store| {
        if store.preferences.sources.iter().any(|s| s.url == url) {
            return Err("该订阅已存在".to_string());
        }
        let display = if name.trim().is_empty() {
            format!("订阅 {}", store.preferences.sources.len() + 1)
        } else {
            name.trim().to_string()
        };
        store.preferences.sources.push(crate::logic::model::Source {
            id: crate::logic::model::random_hex(16),
            name: display,
            url,
        });
        store.save_preferences();
        Ok(())
    })
}

pub fn remove_source(shared: &Shared, id: &str) {
    with_store(shared, |store| {
        if !store.can_change_sources() {
            return;
        }
        store.preferences.sources.retain(|s| s.id != id);
        store.save_preferences();
        let cache = paths::data_root().join(format!("subscription-{id}.yaml"));
        let _ = std::fs::remove_file(cache);
        store.nodes = subscription_service::cached(&store.preferences.sources);
        let nodes = store.nodes.clone();
        let _ = store.aggregate.update(&nodes);
    });
}

pub fn export_yaml(shared: &Shared) {
    with_store(shared, |store| {
        match configuration::export(&store.nodes)
            .and_then(|data| {
                let path = paths::data_root().join("smartclash-aggregate.yaml");
                paths::write_private(&path, &data)?;
                Ok(path)
            }) {
            Ok(path) => {
                store.log(&format!("已导出聚合配置：{}", path.display()));
                paths::open_in_explorer(&path);
            }
            Err(error) => store.error = Some(error),
        }
    });
}

// ---------------------------------------------------------------- 规则

pub fn update_rules(shared: &Shared, ctx: &egui::Context) {
    perform(shared, ctx, |store, _ctx| {
        if let Err(error) = do_update_rules(store) {
            with_store(store, |state| {
                state.rule_status = "规则更新失败，已保留旧规则，稍后自动重试".into();
            });
            fail(store, error);
        }
    });
}

fn do_update_rules(shared: &Shared) -> Result<(), String> {
    let (running, secret, kind) = with_store(shared, |store| {
        (
            store.running,
            store.preferences.secret.clone(),
            store.preferences.kernel,
        )
    });
    if !running {
        return Err("请先启动内核，再更新规则".into());
    }
    with_store(shared, |store| {
        store.rule_status = "正在更新规则…".into();
    });
    match kind.capabilities().rules {
        RulesSupport::Provider => {
            CoreClient::new(kind, &secret).update_rule_providers()?;
        }
        RulesSupport::Restart => {
            // sing-box 的规则集由内核自行下载；重启后立即检查一次更新。
            restart_core(shared, None, true)?;
        }
        RulesSupport::Builtin => {
            with_store(shared, |store| {
                store.rule_status = "Xray 使用内置 geo 数据，不支持在线更新".into();
            });
            return Err("Xray 使用内置 geo 数据（geosite/geoip），无法在线更新规则".into());
        }
    }
    let updated_at = paths::time_text();
    log(shared, &format!("规则已更新 · {updated_at}"));
    with_store(shared, |store| {
        store.rule_status = format!("规则已更新 · {updated_at}");
    });
    Ok(())
}

// ---------------------------------------------------------------- 启动 / 停止

pub fn start_only(shared: &Shared, ctx: &egui::Context) {
    perform(shared, ctx, |store, _ctx| {
        if let Err(error) = do_start(store, false, None, Purpose::Connection) {
            fail(store, error);
        }
    });
}

pub fn stop_all(shared: &Shared, ctx: &egui::Context) {
    {
        let mut store = shared.lock().unwrap();
        if store.stopping {
            return;
        }
        store.stopping = true;
        store.epoch = store.epoch.wrapping_add(1);
    }
    let shared = Arc::clone(shared);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        with_store(&shared, |store| store.busy = true);
        if let Err(error) = do_stop(&shared) {
            fail(&shared, error);
        }
        with_store(&shared, |store| {
            store.busy = false;
            store.stopping = false;
        });
        ctx.request_repaint();
    });
}

fn do_start(
    shared: &Shared,
    tun: bool,
    preferred: Option<String>,
    purpose: Purpose,
) -> Result<(), String> {
    // 串行化内核启停，避免“停止”与“启动”并发时留下脱管进程。
    let (lifecycle, epoch) = with_store(shared, |s| (Arc::clone(&s.lifecycle), s.epoch));
    let _guard = lifecycle.lock().unwrap();
    if with_store(shared, |s| s.stopping || s.epoch != epoch) {
        return Err("操作已停止".into());
    }
    if with_store(shared, |s| s.recovery) {
        return Err("请先恢复上次网络设置".into());
    }
    let (nodes, secret, plan, latencies, traffic_mode, kind) = with_store(shared, |store| {
        (
            store.nodes.clone(),
            store.preferences.secret.clone(),
            store.route_plan.clone(),
            store.latencies.clone(),
            store.preferences.traffic_mode,
            store.preferences.kernel,
        )
    });
    if tun && !kind.capabilities().tun {
        return Err(format!(
            "{} 内核不支持 TUN，请关闭 TUN 或改用 Mihomo / sing-box",
            kind.title()
        ));
    }
    if nodes.is_empty() && traffic_mode.uses_proxy() {
        return Err("请先添加并更新订阅".into());
    }
    let route_nodes = if !traffic_mode.uses_proxy() {
        // 直连模式不选节点，出口固定为 DIRECT。
        Vec::new()
    } else if plan.candidates.is_empty() {
        nodes.clone()
    } else {
        plan.candidates.clone()
    };
    let ordered = NodeRanking::connection_candidates(&route_nodes, &latencies, now_ms());
    let known = ordered.first().and_then(|node| {
        latencies
            .get(&node.id)
            .filter(|l| l.reachable() && l.is_fresh(now_ms(), 300_000))
            .map(|_| node.name.clone())
    });
    let preferred_name = preferred.or(known.clone());
    let outbound = network::default_interface(tun)?;
    let routes = if plan.candidates.is_empty() || !traffic_mode.uses_proxy() {
        None
    } else {
        Some(plan.candidates.as_slice())
    };
    let data = configuration::make_for(
        kind,
        &nodes,
        &secret,
        tun,
        routes,
        &plan.definitions,
        outbound.as_deref(),
        preferred_name.as_deref(),
        traffic_mode,
    )?;
    for warning in &data.warnings {
        log(shared, warning);
    }
    let data = data.data;

    let mut core = with_store(shared, |store| std::mem::take(&mut store.core));
    core.set_kind(kind);
    let client = CoreClient::new(kind, &secret);
    let result = core.start(&data, tun, &client);
    let recovery = core.recovery_needed() || SystemProxy::has_backup();
    if with_store(shared, |s| s.epoch != epoch) {
        let _ = core.stop();
        with_store(shared, |s| {
            s.core = core;
            s.running = false;
            s.tun = false;
        });
        return Err("操作已停止".into());
    }
    with_store(shared, |store| store.core = core);

    match result {
        Ok(()) => {
            let selected = if traffic_mode.uses_proxy() {
                first_supported_name(kind, &route_nodes, preferred_name.as_deref())
            } else {
                String::new()
            };
            with_store(shared, |store| {
                store.running = true;
                store.tun = tun;
                store.selected = selected;
                if traffic_mode.uses_proxy() {
                    store.internet_status = "内核已启动，尚未验证出网".into();
                    store.openai_status = "OpenAI 尚未检测".into();
                } else {
                    store.internet_status = "直连模式已开启，尚未验证出网".into();
                    store.openai_status = "直连模式不检测代理节点".into();
                    store.active_mode = None;
                }
                store.failures = 0;
                store.last_switch = 0;
                store.last_openai_probe = 0;
                store.last_openai_failover = 0;
                store.publish_route();
                store.log(match purpose {
                    Purpose::LatencyTest => "仅测速内核已启动，未接管系统网络",
                    Purpose::Connection if !traffic_mode.uses_proxy() => "直连模式内核已启动",
                    Purpose::Connection if tun => "TUN 已启动",
                    Purpose::Connection => "代理内核已启动",
                });
                store.log(&format!("当前内核：{}", kind.title()));
            });
            start_traffic(shared);
            start_activity(shared);
            if purpose == Purpose::Connection {
                start_monitor(shared);
            }
            Ok(())
        }
        Err(error) => {
            with_store(shared, |store| store.recovery = recovery);
            Err(error)
        }
    }
}

fn do_stop(shared: &Shared) -> Result<(), String> {
    let lifecycle = with_store(shared, |s| Arc::clone(&s.lifecycle));
    let _guard = lifecycle.lock().unwrap();
    if let Some(flag) = with_store(shared, |store| store.latency_cancel.take()) {
        flag.store(true, Ordering::SeqCst);
    }
    if let Some(flag) = with_store(shared, |store| store.monitor_stop.take()) {
        flag.store(true, Ordering::SeqCst);
    }
    if let Some(flag) = with_store(shared, |store| store.traffic_stop.take()) {
        flag.store(true, Ordering::SeqCst);
    }
    if let Some(flag) = with_store(shared, |store| store.activity_stop.take()) {
        flag.store(true, Ordering::SeqCst);
    }
    let activity = with_store(shared, |store| Arc::clone(&store.activity));
    crate::state::activity::stop(&activity);

    // 必须先恢复系统代理，再移除它指向的本地端点；
    // 即使代理恢复失败也要继续停内核——TUN 劫持比代理残留严重得多。
    let mut errors: Vec<String> = Vec::new();
    if let Err(error) = SystemProxy::restore() {
        errors.push(error);
    }

    let mut core = with_store(shared, |store| std::mem::take(&mut store.core));
    let result = core.stop();
    let recovery = core.recovery_needed();
    if let Err(error) = result {
        errors.push(error);
    }
    with_store(shared, |store| store.core = core);

    with_store(shared, |store| {
        // 恢复失败时系统代理可能仍指向已停止的本地端点，用备份是否还在如实反映。
        store.system_proxy = SystemProxy::has_backup();
        store.running = false;
        store.tun = false;
        store.selected.clear();
        store.dynamic_mode = false;
        store.route_plan = RoutePlan::default();
        store.routing_status = "尚未打通".into();
        store.internet_status = "未连接".into();
        store.active_mode = None;
        store.recovery = recovery || !errors.is_empty();
        store.publish_route();
        if errors.is_empty() {
            store.log("代理已停止，网络设置已恢复");
        } else {
            store.log(&format!("停止未完全成功：{}", errors.join("；")));
        }
    });

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

pub fn toggle_system(shared: &Shared, ctx: &egui::Context) {
    perform(shared, ctx, |store, _ctx| {
        if let Err(error) = do_toggle_system(store) {
            fail(store, error);
        }
    });
}

fn do_toggle_system(shared: &Shared) -> Result<(), String> {
    let epoch = with_store(shared, |s| s.epoch);
    if with_store(shared, |s| s.system_proxy) {
        SystemProxy::restore()?;
        with_store(shared, |store| {
            store.system_proxy = false;
            if !store.tun {
                store.dynamic_mode = false;
                store.routing_status = "系统代理已关闭，内核继续运行".into();
            }
            store.log("系统代理已关闭");
        });
        return Ok(());
    }
    let started_here = !with_store(shared, |s| s.running);
    if started_here {
        if let Err(error) = do_start(shared, false, None, Purpose::Connection) {
            return Err(error);
        }
    }
    if cancelled(shared, epoch) {
        if started_here {
            let _ = do_stop(shared);
        }
        return Err("操作已停止".into());
    }
    let result = do_ensure_route(shared, true, Some(epoch))
        .and_then(|_| enable_system_proxy(shared, epoch));
    match result {
        Ok(()) => {
            with_store(shared, |store| {
                store.log("系统 HTTP / HTTPS 代理已开启");
            });
            start_monitor(shared);
            Ok(())
        }
        Err(error) => {
            if started_here {
                let _ = do_stop(shared);
            }
            with_store(shared, |store| {
                store.recovery = SystemProxy::has_backup();
            });
            Err(error)
        }
    }
}

fn enable_system_proxy(shared: &Shared, epoch: u64) -> Result<(), String> {
    // Serialize the registry change with stop/start so a cancelled operation
    // cannot leave the system proxy pointing at an endpoint that was removed.
    let lifecycle = with_store(shared, |store| Arc::clone(&store.lifecycle));
    let _guard = lifecycle.lock().unwrap();
    if cancelled(shared, epoch) {
        return Err("操作已停止".into());
    }
    let result = SystemProxy::enable();
    with_store(shared, |store| {
        store.system_proxy = SystemProxy::has_backup();
        if result.is_err() && store.system_proxy {
            store.recovery = true;
        }
    });
    result
}

pub fn toggle_tun(shared: &Shared, ctx: &egui::Context) {
    perform(shared, ctx, |store, _ctx| {
        if let Err(error) = do_toggle_tun(store) {
            fail(store, error);
        }
    });
}

fn do_toggle_tun(shared: &Shared) -> Result<(), String> {
    let kind = kernel_of(shared);
    if !kind.capabilities().tun {
        return Err(format!(
            "{} 内核不支持 TUN，请改用 Mihomo 或 sing-box",
            kind.title()
        ));
    }
    let epoch = with_store(shared, |s| s.epoch);
    let (tun, system_proxy, dynamic, previous, plan) = with_store(shared, |store| {
        (
            store.tun,
            store.system_proxy,
            store.dynamic_mode,
            store.selected.clone(),
            store.route_plan.clone(),
        )
    });
    if tun {
        return tun_transition::restart(
            &mut TunRestart { shared, epoch, dynamic, selected: previous, plan },
            false,
            system_proxy,
        );
    }

    // 先在非特权状态验证单跳出口，再让内核接管系统流量。
    if !with_store(shared, |s| s.running) {
        do_start(shared, false, None, Purpose::Connection)?;
    }
    do_ensure_route(shared, true, Some(epoch))?;
    if cancelled(shared, epoch) {
        return Err("操作已停止".into());
    }
    let (verified, plan) = with_store(shared, |store| (store.selected.clone(), store.route_plan.clone()));
    tun_transition::restart(
        &mut TunRestart { shared, epoch, dynamic, selected: verified, plan },
        true,
        system_proxy,
    )
}

struct TunRestart<'a> {
    shared: &'a Shared,
    epoch: u64,
    dynamic: bool,
    selected: String,
    plan: RoutePlan,
}

impl TunRuntime for TunRestart<'_> {
    fn stop(&mut self) -> Result<(), String> {
        do_stop(self.shared)
    }

    fn start(&mut self, tun: bool) -> Result<(), String> {
        if cancelled(self.shared, self.epoch) {
            return Err("操作已停止".into());
        }
        with_store(self.shared, |store| {
            store.dynamic_mode = self.dynamic;
            store.route_plan = self.plan.clone();
        });
        do_start(self.shared, tun, Some(self.selected.clone()), Purpose::Connection)
    }

    fn verify_tun(&mut self) -> Result<(), String> {
        verify_tun(self.shared, self.epoch)
    }

    fn enable_system_proxy(&mut self) -> Result<(), String> {
        enable_system_proxy(self.shared, self.epoch)
    }
}

fn verify_tun(shared: &Shared, epoch: u64) -> Result<(), String> {
    let (secret, kind) = with_store(shared, |s| {
        (s.preferences.secret.clone(), s.preferences.kernel)
    });
    let client = CoreClient::new(kind, &secret);
    // 先确认 wintun 网卡与分流路由真的接管了系统流量；
    // 只探测 7890 混合端口无法区分「TUN 生效」和「仅本地代理可用」。
    let mut tun_detail = String::new();
    for attempt in 1..=8u32 {
        if cancelled(shared, epoch) {
            return Err("操作已停止".into());
        }
        match network::tun_ready() {
            Ok(detail) => {
                tun_detail = detail;
                break;
            }
            Err(_) => {
                with_store(shared, |store| {
                    store.internet_status =
                        format!("TUN 内核已启动，等待网卡接管（{attempt}/8）");
                });
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
    if tun_detail.is_empty() {
        return Err(
            "TUN 内核已启动，但未检测到已接管流量的 TUN 网卡/路由；请检查 wintun、管理员权限或其他 VPN 冲突"
                .into(),
        );
    }

    // TUN 接管后路由/接口就绪与出口建连需要时间，且出口线路可能抖动，
    // 给约 10 秒重试窗口，避免刚接管就误判失败。
    let mut result = client.internet_probe(proxy_port());
    for attempt in 1..=5u32 {
        if result.health == crate::logic::model::Health::Healthy {
            break;
        }
        if cancelled(shared, epoch) {
            return Err("操作已停止".into());
        }
        with_store(shared, |store| {
            store.internet_status = format!("TUN 已接管，正在验证出网（{attempt}/5）");
        });
        std::thread::sleep(std::time::Duration::from_secs(2));
        result = client.internet_probe(proxy_port());
    }
    if result.health != crate::logic::model::Health::Healthy {
        return Err(
            "TUN 接管后出网验证失败；请检查出口节点状态、DNS、物理网卡或其他 VPN 冲突".into(),
        );
    }
    with_store(shared, |store| {
        store.internet_status = format!("TUN 出网已验证 · {} ms", result.milliseconds);
        store.routing_status = "已打通".into();
        store.log(&format!("TUN 已接管：{tun_detail}"));
    });
    Ok(())
}

pub fn connect(shared: &Shared, ctx: &egui::Context) {
    perform(shared, ctx, |store, _ctx| {
        if let Err(error) = do_connect(store) {
            fail(store, error);
        }
    });
}

fn do_connect(shared: &Shared) -> Result<(), String> {
    let epoch = with_store(shared, |s| s.epoch);
    if with_store(shared, |s| s.recovery) {
        let _ = do_stop(shared);
    }
    if with_store(shared, |s| s.running) {
        do_stop(shared)?;
    }
    if with_store(shared, |s| s.nodes.is_empty())
        && with_store(shared, |s| s.preferences.traffic_mode).uses_proxy()
    {
        do_refresh(shared)?;
    }
    if cancelled(shared, epoch) {
        return Err("操作已停止".into());
    }
    let nodes = with_store(shared, |s| s.nodes.clone());
    with_store(shared, |store| {
        store.search_round = 0;
        store.dynamic_mode = true;
        store.route_plan = RoutePlan {
            candidates: nodes,
            definitions: Vec::new(),
        };
        store.save_preferences();
    });
    let result = do_start(shared, false, None, Purpose::Connection)
        .and_then(|_| do_ensure_route(shared, true, Some(epoch)))
        .and_then(|_| {
            if !with_store(shared, |s| s.system_proxy || s.tun) {
                SystemProxy::enable()?;
                with_store(shared, |store| {
                    store.system_proxy = true;
                    store.routing_status = "已打通".into();
                });
            }
            Ok(())
        });
    if let Err(error) = result {
        let _ = do_stop(shared);
        return Err(error);
    }
    Ok(())
}

pub fn apply_mode(shared: &Shared, ctx: &egui::Context) {
    perform(shared, ctx, |store, _ctx| {
        if !with_store(store, |s| s.running) {
            fail(store, "请先连接，或启动本机内核");
            return;
        }
        let epoch = with_store(store, |s| s.epoch);
        let previous = with_store(store, |s| s.routing_status.clone());
        if let Err(error) = do_ensure_route(store, false, Some(epoch)) {
            with_store(store, |s| s.routing_status = previous);
            fail(store, error);
        }
    });
}

// ---------------------------------------------------------------- 流量模式

pub fn change_traffic_mode(shared: &Shared, ctx: &egui::Context, mode: TrafficMode) {
    perform(shared, ctx, move |store, _ctx| {
        if let Err(error) = do_change_traffic_mode(store, mode) {
            fail(store, error);
        }
    });
}

fn do_change_traffic_mode(shared: &Shared, wanted: TrafficMode) -> Result<(), String> {
    let epoch = with_store(shared, |s| s.epoch);
    let (current, running, recovery, node_count, secret, kind) = with_store(shared, |store| {
        (
            store.preferences.traffic_mode,
            store.running,
            store.recovery,
            store.nodes.len(),
            store.preferences.secret.clone(),
            store.preferences.kernel,
        )
    });
    if wanted == current {
        return Ok(());
    }
    if recovery {
        return Err("请先恢复上次网络设置".into());
    }
    if running && wanted.uses_proxy() && node_count == 0 {
        return Err("请先添加并更新订阅，才能使用规则或全局代理".into());
    }

    // sing-box / Xray 的流量模式写在完整配置里，必须重启内核才能生效。
    if running && kind != KernelKind::Mihomo {
        with_store(shared, |store| {
            store.preferences.traffic_mode = wanted;
            store.save_preferences();
        });
        if let Err(error) = restart_core(shared, None, true) {
            with_store(shared, |store| {
                store.preferences.traffic_mode = current;
                store.save_preferences();
            });
            let restored = restart_core(shared, None, true).is_ok();
            with_store(shared, |store| {
                if !restored {
                    store.recovery = true;
                    store.log("流量模式回滚未确认，请停止并恢复网络");
                }
            });
            return Err(error);
        }
        with_store(shared, |store| {
            match wanted {
                TrafficMode::Direct => {
                    store.selected.clear();
                    store.active_mode = None;
                    store.failures = 0;
                    store.routing_status = "直连模式".into();
                    store.internet_status = "直连模式已开启，等待验证".into();
                    store.openai_status = "直连模式不检测代理节点".into();
                }
                _ => {
                    store.failures = 0;
                    store.routing_status = "流量模式已切换，等待验证".into();
                    store.internet_status = "模式已切换，尚未验证出网".into();
                    store.openai_status = "OpenAI 尚未检测".into();
                }
            }
            store.publish_route();
            store.log(&format!(
                "流量模式：{}（已重启 {} 内核生效）",
                wanted.title(),
                kind.title()
            ));
        });
        start_monitor(shared);
        return Ok(());
    }

    let client = CoreClient::new(kind, &secret);
    if running {
        if let Err(error) = client.set_traffic_mode(wanted) {
            // 切换失败时尽力回到原模式，避免界面与内核状态不一致。
            let restored = client.set_traffic_mode(current).is_ok();
            with_store(shared, |store| {
                if !restored {
                    store.recovery = true;
                    store.log("流量模式回滚未确认，请停止并恢复网络");
                }
            });
            return Err(error);
        }
    }
    with_store(shared, |store| {
        store.preferences.traffic_mode = wanted;
        store.save_preferences();
        match (wanted, running) {
            (TrafficMode::Direct, true) => {
                store.selected.clear();
                store.active_mode = None;
                store.failures = 0;
                store.routing_status = "直连模式".into();
                store.internet_status = "直连模式已开启，等待验证".into();
                store.openai_status = "直连模式不检测代理节点".into();
            }
            (TrafficMode::Direct, false) => {
                store.selected.clear();
                store.active_mode = None;
                store.routing_status = "尚未打通".into();
                store.internet_status = "未连接".into();
                store.openai_status = "直连模式不检测代理节点".into();
            }
            (_, true) => {
                store.failures = 0;
                store.routing_status = "流量模式已切换，等待验证".into();
                store.internet_status = "模式已切换，尚未验证出网".into();
                store.openai_status = "OpenAI 尚未检测".into();
            }
            (_, false) => {
                store.routing_status = "尚未打通".into();
            }
        }
        store.publish_route();
        store.log(&format!("流量模式：{}", wanted.title()));
    });
    if running {
        // 已建立的连接会沿用切换前的路由，必须断开重建。
        if let Err(error) = client.close_connections() {
            log(shared, &format!("模式已切换，旧连接关闭未完成：{error}"));
        }
        // 从直连切回代理时 selected 已被清空，必须重新选路；
        // 若当前节点已从订阅中删除，同样需要重新选择。
        let needs_route = with_store(shared, |store| {
            let nodes = store.node_list_nodes();
            !nodes.iter().any(|node| node.name == store.selected)
        });
        if wanted.uses_proxy() && needs_route {
            if let Err(error) = do_ensure_route(shared, true, Some(epoch)) {
                return Err(format!("流量模式已切换，但重新选路失败：{error}"));
            }
        }
        start_monitor(shared);
    }
    Ok(())
}

// ---------------------------------------------------------------- 选路

fn do_ensure_route(
    shared: &Shared,
    prefer_current: bool,
    epoch: Option<u64>,
) -> Result<(), String> {
    let paused = |shared: &Shared| {
        with_store(shared, |store| {
            store.stopping || epoch.map(|value| store.epoch != value).unwrap_or(false)
        })
    };
    if paused(shared) {
        return Err("操作已停止".into());
    }
    // 直连模式不选路：只要本地代理端口能直连出网，就可以继续开系统代理 / TUN。
    if !with_store(shared, |store| store.preferences.traffic_mode).uses_proxy() {
        let client = client_of(shared);
        let result = client.internet_probe(proxy_port());
        if result.health != crate::logic::model::Health::Healthy {
            return Err("直连模式出网验证失败，请检查本机网络；未开启新的系统代理".into());
        }
        with_store(shared, |store| {
            store.internet_status = format!("直连模式出网已验证 · {} ms", result.milliseconds);
            store.routing_status = "直连模式已开启".into();
            store.openai_status = "直连模式不检测代理节点".into();
            store.failures = 0;
        });
        return Ok(());
    }
    let (mode, selected, secret) = with_store(shared, |store| {
        (
            store.preferences.mode(),
            store.selected.clone(),
            store.preferences.secret.clone(),
        )
    });
    let mut latencies = with_store(shared, |store| store.latencies.clone());
    let (nodes, mut measurements) = with_store(shared, |store| {
        (store.node_list_nodes(), store.measurements.clone())
    });
    with_store(shared, |store| {
        store.routing_status = format!("正在验证{}", mode.title());
    });

    // 一键连接最怕把时间耗在同一台故障服务器上：先当前线路，再 OpenAI 已验证节点，
    // 然后按订阅顺序补齐。公告、流量类信息节点不参与拨号。
    let mut candidates: Vec<ProxyNode> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if prefer_current {
        if let Some(node) = nodes.iter().find(|n| n.name == selected) {
            remember_candidate(&mut candidates, &mut seen, node);
        }
    }
    if mode == RoutingMode::Ai {
        for node in &nodes {
            if measurements
                .get(&node.id)
                .and_then(|m| m.openai_reachable)
                .unwrap_or(false)
            {
                remember_candidate(&mut candidates, &mut seen, node);
            }
        }
    }
    for node in &nodes {
        remember_candidate(&mut candidates, &mut seen, node);
    }
    if candidates.is_empty() {
        return Err("没有可用链路".into());
    }

    // 第一轮每个入口主机只取一个节点，十几秒内就能覆盖全部订阅商。
    let mut quick: Vec<ProxyNode> = Vec::new();
    let mut hosts: HashSet<String> = HashSet::new();
    for node in &candidates {
        if hosts.insert(endpoint_host(node)) {
            quick.push(node.clone());
        }
        if quick.len() >= 16 {
            break;
        }
    }

    let client = client_of(shared);
    let deadline = now_ms() + 45_000;
    let mut tested: HashSet<String> = HashSet::new();
    let mut speed_attempts = 0usize;
    // AI 模式全都没有通过 OpenAI 预检时的兜底线路，最后再普通出网验证。
    let mut ai_fallback: Option<ProxyNode> = None;

    for source in [quick, candidates] {
        let pending: Vec<ProxyNode> = source
            .into_iter()
            .filter(|node| !tested.contains(&node.id))
            .collect();
        let mut offset = 0usize;
        while offset < pending.len() && now_ms() < deadline {
            if paused(shared) {
                return Err("操作已停止".into());
            }
            let end = (offset + 18).min(pending.len());
            let batch: Vec<ProxyNode> = pending[offset..end].to_vec();
            offset = end;
            for node in &batch {
                tested.insert(node.id.clone());
            }
            with_store(shared, |store| {
                store.routing_status =
                    format!("正在验证{} · 已测试 {} 个入口", mode.title(), tested.len());
            });
            measure_latencies(shared, &batch);
            if paused(shared) {
                return Err("操作已停止".into());
            }
            latencies = with_store(shared, |store| store.latencies.clone());

            let mut reachable: Vec<ProxyNode> = NodeRanking::sorted(
                &batch
                    .iter()
                    .filter(|n| latencies.get(&n.id).map(|l| l.reachable()).unwrap_or(false))
                    .cloned()
                    .collect::<Vec<_>>(),
                &latencies,
                crate::logic::model::NodeSortOrder::Latency,
            );

            // AI 模式优先使用 OpenAI 已验证节点；没有缓存时先按延迟顺序做一次预检，
            // 避免把香港等被地区限制的出口直接选成业务线路。
            if mode == RoutingMode::Ai {
                reachable.sort_by_key(|node| {
                    !measurements
                        .get(&node.id)
                        .and_then(|m| m.openai_reachable)
                        .unwrap_or(false)
                });
            }

            let mut eligible = reachable.clone();
            if mode == RoutingMode::Ai {
                let verify_deadline = deadline.min(now_ms() + AI_VERIFY_BUDGET_MS);
                let verified = verify_ai_candidates(
                    shared,
                    &reachable,
                    &measurements,
                    verify_deadline,
                    AI_VERIFY_LIMIT,
                    &paused,
                );
                measurements = with_store(shared, |store| store.measurements.clone());
                let Some(verified) = verified else {
                    if ai_fallback.is_none() {
                        ai_fallback = reachable.first().cloned();
                    }
                    // 本批没有通过 OpenAI 验证的候选，继续验证下一批；
                    // 所有批次都失败后才回退到普通出网线路。
                    continue;
                };
                // AI 模式只尝试 OpenAI 已验证的候选，避免预检节点业务出网失败后
                // 又顺手连回同批未验证的香港出口。
                let now = now_ms();
                eligible = reachable
                    .iter()
                    .filter(|node| {
                        node.id == verified.id
                            || recently_verified(measurements.get(&node.id), now)
                    })
                    .cloned()
                    .collect();
                eligible.sort_by_key(|node| node.id != verified.id);
            }
            if mode == RoutingMode::HighSpeed {
                with_store(shared, |store| {
                    store.routing_status = "正在抽测下载吞吐（每轮约 1 MiB）".into();
                });
                for node in &reachable {
                    if speed_attempts >= 3 || now_ms() >= deadline {
                        break;
                    }
                    speed_attempts += 1;
                    let sample = with_probe(shared, node, |port| {
                        let client = CoreClient::new(kernel_of(shared), &secret);
                        client.throughput(port)
                    })?;
                    with_store(shared, |store| {
                        let mut measured =
                            store.measurements.get(&node.id).cloned().unwrap_or_default();
                        measured.throughput = sample;
                        store.measurements.insert(node.id.clone(), measured);
                    });
                }
                let measured = with_store(shared, |s| s.measurements.clone());
                let ranked = ModeRanking::best(&reachable, mode, &latencies, &measured);
                if !ranked.is_empty() {
                    eligible = ranked;
                }
            }
            for node in eligible {
                if paused(shared) {
                    return Err("操作已停止".into());
                }
                let previous = with_store(shared, |store| {
                    store
                        .route_nodes()
                        .into_iter()
                        .find(|n| n.name == store.selected)
                });
                do_select(shared, &node)?;
                let result = client.internet_probe(proxy_port());
                if result.health == crate::logic::model::Health::Healthy {
                    mark_connected(shared, &node, &result, mode);
                    return Ok(());
                }
                if let Some(previous) = previous {
                    let _ = do_select(shared, &previous);
                }
                latencies.insert(
                    node.id.clone(),
                    NodeLatency::new(None, Some("业务出网未通过".into())),
                );
                with_store(shared, |store| {
                    store.latencies = latencies.clone();
                });
                if now_ms() >= deadline {
                    break;
                }
            }
            if mode == RoutingMode::HighSpeed && speed_attempts >= 3 {
                break;
            }
        }
    }

    // AI 模式所有候选都没通过 OpenAI 预检时，仍然允许先连上普通出网线路；
    // 后台监控会继续复检并切换，避免用户完全无法上网。
    if mode == RoutingMode::Ai {
        if let Some(node) = ai_fallback {
            if !paused(shared) && now_ms() < deadline {
                do_select(shared, &node)?;
                let result = client.internet_probe(proxy_port());
                if result.health == crate::logic::model::Health::Healthy {
                    mark_connected(shared, &node, &result, mode);
                    log(shared, "没有候选通过 OpenAI 预检，已连接普通线路并继续后台复检");
                    return Ok(());
                }
                latencies.insert(
                    node.id.clone(),
                    NodeLatency::new(None, Some("业务出网未通过".into())),
                );
                with_store(shared, |store| {
                    store.latencies = latencies.clone();
                });
            }
        }
    }

    with_store(shared, |store| {
        store.save_latencies();
        store.save_measurements();
        store.routing_status = format!("{}未通过验证", mode.title());
    });
    Err(format!(
        "已测试 {} 个入口，均没有通过{}验证；未开启新的系统代理，请检查订阅是否过期或稍后重试",
        tested.len(),
        mode.title()
    ))
}

/// 记录一次成功的业务线路验证。调用方已经切换并完成 HTTPS 出网探测。
fn mark_connected(shared: &Shared, node: &ProxyNode, result: &ProbeResult, mode: RoutingMode) {
    with_store(shared, |store| {
        store.internet_status = format!("HTTPS 出网已验证 · {} ms", result.milliseconds);
        store.routing_status = "已打通".into();
        store.active_mode = Some(mode);
        store.set_status(&node.id, result.clone());
        if mode == RoutingMode::Ai {
            let confirmed = store
                .measurements
                .get(&node.id)
                .and_then(|m| m.openai_reachable)
                .unwrap_or(false);
            store.openai_status = if confirmed {
                "OpenAI 已验证".into()
            } else {
                "OpenAI 未确认 · 已连接普通线路".into()
            };
        }
        store.save_latencies();
        store.save_measurements();
    });
}

/// AI 模式连接前，按延迟顺序最多复检 `limit` 个候选的 OpenAI 可达性。
/// 60 秒内刚被确认地区限制的节点直接跳过，避免反复连接时重复撞同一批香港出口。
/// 返回第一个通过验证的节点；全部失败时返回 None，由调用方决定是否回退普通线路。
fn verify_ai_candidates(
    shared: &Shared,
    candidates: &[ProxyNode],
    measurements: &HashMap<String, LinkMeasurements>,
    deadline: u64,
    limit: usize,
    paused: &impl Fn(&Shared) -> bool,
) -> Option<ProxyNode> {
    let mut attempts = 0usize;
    for node in candidates {
        if attempts >= limit || now_ms() >= deadline || paused(shared) {
            break;
        }
        if recently_blocked(measurements.get(&node.id), now_ms()) {
            continue;
        }
        attempts += 1;
        with_store(shared, |store| {
            store.routing_status = format!(
                "正在验证 OpenAI 可达性 · 第 {attempts} 个候选 · {}",
                node.name
            );
        });
        match do_check(shared, node, true) {
            Ok(result) if result.health == crate::logic::model::Health::Healthy => {
                return Some(node.clone())
            }
            Ok(result) => {
                if result.health == crate::logic::model::Health::Blocked {
                    log(shared, &format!("{} 被 OpenAI 地区限制，跳过", node.name));
                }
            }
            Err(error) => log(shared, &format!("{} OpenAI 预检失败：{error}", node.name)),
        }
    }
    None
}

fn recently_blocked(measured: Option<&LinkMeasurements>, now: u64) -> bool {
    match measured {
        Some(m) if m.openai_reachable == Some(false) => m
            .openai_checked
            .map(|checked| now.saturating_sub(checked) <= 60_000)
            .unwrap_or(false),
        _ => false,
    }
}

fn recently_verified(measured: Option<&LinkMeasurements>, now: u64) -> bool {
    match measured {
        Some(m) if m.openai_reachable == Some(true) => m
            .openai_checked
            .map(|checked| now.saturating_sub(checked) <= 15 * 60_000)
            .unwrap_or(false),
        _ => false,
    }
}

fn remember_candidate(
    list: &mut Vec<ProxyNode>,
    seen: &mut HashSet<String>,
    node: &ProxyNode,
) {
    if is_informational(&node.name) {
        return;
    }
    if seen.insert(node.id.clone()) {
        list.push(node.clone());
    }
}

fn is_informational(name: &str) -> bool {
    const MARKS: [&str; 8] = ["剩余", "流量", "到期", "重置", "公告", "官网", "客服", "套餐"];
    MARKS.iter().any(|mark| name.contains(mark))
}

fn endpoint_host(node: &ProxyNode) -> String {
    node.payload
        .get("server")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| node.name.clone())
}

fn do_select(shared: &Shared, node: &ProxyNode) -> Result<(), String> {
    let (running, plan, secret, selected, tun, traffic_mode, kind) =
        with_store(shared, |store| {
            (
                store.running,
                store.route_plan.clone(),
                store.preferences.secret.clone(),
                store.selected.clone(),
                store.tun,
                store.preferences.traffic_mode,
                store.preferences.kernel,
            )
        });
    if !running {
        return Err("请先启动内核".into());
    }
    if !traffic_mode.uses_proxy() {
        return Err("当前是直连模式，不切换节点；请先切换到规则或全局代理".into());
    }
    let in_plan = plan.candidates.iter().any(|n| n.id == node.id);
    if !in_plan {
        let (nodes, definitions) = with_store(shared, |store| {
            (store.nodes.clone(), store.route_plan.definitions.clone())
        });
        let mut candidates = plan.candidates.clone();
        candidates.push(node.clone());
        if kind == KernelKind::Mihomo {
            let built = configuration::make_for(
                kind,
                &nodes,
                &secret,
                tun,
                Some(&candidates),
                &definitions,
                None,
                Some(&selected),
                traffic_mode,
            )?;
            let text = String::from_utf8_lossy(&built.data).to_string();
            CoreClient::new(kind, &secret).reload_config(&text)?;
            with_store(shared, |store| {
                store.route_plan = RoutePlan {
                    candidates,
                    definitions,
                };
            });
        } else {
            // sing-box / Xray 只能通过重启内核把新节点写进配置。
            with_store(shared, |store| {
                store.route_plan = RoutePlan {
                    candidates,
                    definitions,
                };
            });
            restart_core(shared, Some(node.name.clone()), true)?;
        }
    } else if kind == KernelKind::Xray {
        restart_core(shared, Some(node.name.clone()), true)?;
    } else {
        CoreClient::new(kind, &secret).select(&node.name, "SMART")?;
    }
    with_store(shared, |store| {
        store.selected = node.name.clone();
        store.last_switch = now_ms();
        store.failures = 0;
        store.log(&format!("切换至 {}", node.name));
        store.publish_route();
    });
    Ok(())
}

fn with_probe<T>(
    shared: &Shared,
    node: &ProxyNode,
    operation: impl FnOnce(u16) -> T,
) -> Result<T, String> {
    let (secret, kind) = with_store(shared, |s| {
        (s.preferences.secret.clone(), s.preferences.kernel)
    });
    if kind == KernelKind::Xray {
        // Xray 没有 PROBE 组，临时启动单节点内核。
        let probe = core_probe::EphemeralProbe::start(kind, node)?;
        return Ok(operation(probe.port));
    }
    let client = CoreClient::new(kind, &secret);
    client.select(&node.name, "PROBE")?;
    Ok(operation(PROBE_PORT))
}

fn do_check(shared: &Shared, node: &ProxyNode, openai: bool) -> Result<ProbeResult, String> {
    let secret = with_store(shared, |s| s.preferences.secret.clone());
    let result = with_probe(shared, node, |port| {
        let kind = kernel_of(shared);
        let client = CoreClient::new(kind, &secret);
        if openai {
            client.openai_probe(port)
        } else {
            client.internet_probe(port)
        }
    })?;
    with_store(shared, |store| {
        store.set_status(&node.id, result.clone());
        if openai {
            let mut measured = store.measurements.get(&node.id).cloned().unwrap_or_default();
            measured.openai_reachable =
                Some(result.health == crate::logic::model::Health::Healthy);
            measured.openai_checked = Some(now_ms());
            store.measurements.insert(node.id.clone(), measured);
        }
        store.publish_route();
    });
    Ok(result)
}

pub fn select_verified(shared: &Shared, ctx: &egui::Context, node: ProxyNode) {
    perform(shared, ctx, move |store, _ctx| {
        if let Err(error) = do_select_verified(store, node) {
            fail(store, error);
        }
    });
}

fn do_select_verified(shared: &Shared, node: ProxyNode) -> Result<(), String> {
    let (running, secret, kind) = with_store(shared, |s| {
        (
            s.running,
            s.preferences.secret.clone(),
            s.preferences.kernel,
        )
    });
    if !running {
        return Err("请先启动内核或运行一次延迟测试".into());
    }
    if !with_store(shared, |s| s.preferences.traffic_mode).uses_proxy() {
        return Err("当前是直连模式，不切换节点；请先切换到规则或全局代理".into());
    }
    let previous = with_store(shared, |store| {
        store
            .route_nodes()
            .into_iter()
            .find(|n| n.name == store.selected)
    });
    let sample = if kind == KernelKind::Xray {
        core_probe::latency(kind, &node, 5000)
    } else {
        CoreClient::new(kind, &secret).delay(&node.name, 5000)
    };
    with_store(shared, |store| {
        store.latencies.insert(node.id.clone(), sample.clone());
        store.save_latencies();
    });
    if !sample.reachable() {
        return Err("该节点本次 HTTPS 测试未通过，已保留原业务出口".into());
    }
    do_select(shared, &node)?;
    let result = client_of(shared).internet_probe(proxy_port());
    if result.health != crate::logic::model::Health::Healthy {
        if let Some(previous) = previous {
            let _ = do_select(shared, &previous);
        }
        return Err("切换后的业务出网验证未通过，已尝试恢复原出口".into());
    }
    with_store(shared, |store| {
        store.internet_status = format!("HTTPS 出网已验证 · {} ms", result.milliseconds);
        store.openai_status = "OpenAI 尚未检测".into();
    });
    start_monitor(shared);
    Ok(())
}

// ---------------------------------------------------------------- 延迟测试

pub fn begin_latency_all(shared: &Shared, ctx: &egui::Context) {
    let kind = kernel_of(shared);
    let targets = with_store(shared, |s| {
        s.nodes
            .iter()
            .filter(|node| kind.supports_node(node))
            .cloned()
            .collect::<Vec<_>>()
    });
    begin_latency(shared, ctx, targets);
}

pub fn begin_latency_one(shared: &Shared, ctx: &egui::Context, node: ProxyNode) {
    let kind = kernel_of(shared);
    if !kind.supports_node(&node) {
        with_store(shared, |store| {
            store.error = Some(format!(
                "{} 内核不支持该节点协议，请更换内核或选择其他节点",
                kind.title()
            ));
        });
        return;
    }
    begin_latency(shared, ctx, vec![node]);
}

fn begin_latency(shared: &Shared, ctx: &egui::Context, targets: Vec<ProxyNode>) {
    {
        let mut store = shared.lock().unwrap();
        if store.busy || store.is_testing_latency || store.recovery {
            return;
        }
        if targets.is_empty() {
            store.error = Some("请先添加并更新订阅".into());
            return;
        }
        store.is_testing_latency = true;
        store.latency_completed = 0;
        store.latency_total = targets.len();
        store.testing_ids = targets.iter().map(|n| n.id.clone()).collect();
        store.latency_cancel = Some(Arc::new(AtomicBool::new(false)));
    }
    let shared = Arc::clone(shared);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let cancel = with_store(&shared, |s| s.latency_cancel.clone())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let result = (|| -> Result<(), String> {
            if !with_store(&shared, |s| s.running) {
                do_start(&shared, false, None, Purpose::LatencyTest)?;
                with_store(&shared, |store| {
                    store.internet_status = "仅测速内核 · 系统代理 / TUN 未开启".into();
                });
            }
            measure_latencies_cancellable(&shared, &targets, &cancel);
            with_store(&shared, |store| store.save_latencies());
            Ok(())
        })();
        with_store(&shared, |store| {
            store.is_testing_latency = false;
            store.testing_ids.clear();
            store.latency_cancel = None;
            match result {
                Ok(()) => {
                    let cancelled = cancel.load(Ordering::SeqCst);
                    store.log(&if cancelled {
                        "延迟测试已取消，已保留完成的结果".to_string()
                    } else {
                        format!(
                            "延迟测试完成：{}/{}，业务节点未改变",
                            store.latency_completed, store.latency_total
                        )
                    });
                }
                Err(error) => {
                    store.error = Some(error.clone());
                    store.log(&error);
                }
            }
        });
        ctx.request_repaint();
    });
}

pub fn cancel_latency(shared: &Shared) {
    if let Some(flag) = with_store(shared, |s| s.latency_cancel.clone()) {
        flag.store(true, Ordering::SeqCst);
    }
}

fn measure_latencies(shared: &Shared, targets: &[ProxyNode]) {
    let cancel = Arc::new(AtomicBool::new(false));
    measure_latencies_inner(shared, targets, &cancel, false);
}

fn measure_latencies_cancellable(shared: &Shared, targets: &[ProxyNode], cancel: &Arc<AtomicBool>) {
    measure_latencies_inner(shared, targets, cancel, true);
}

fn measure_latencies_inner(
    shared: &Shared,
    targets: &[ProxyNode],
    cancel: &Arc<AtomicBool>,
    track_progress: bool,
) {
    if targets.is_empty() {
        return;
    }
    let (secret, kind) = with_store(shared, |s| {
        (s.preferences.secret.clone(), s.preferences.kernel)
    });
    let next = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel::<(String, NodeLatency)>();
    let concurrency = if kind == KernelKind::Xray {
        3.min(targets.len())
    } else {
        6.min(targets.len())
    };

    std::thread::scope(|scope| {
        for _ in 0..concurrency {
            let tx = tx.clone();
            let secret = secret.clone();
            let cancel = Arc::clone(cancel);
            let next = Arc::clone(&next);
            scope.spawn(move || {
                loop {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    if index >= targets.len() {
                        break;
                    }
                    let latency = if kind == KernelKind::Xray {
                        core_probe::latency(kind, &targets[index], 5000)
                    } else {
                        CoreClient::new(kind, &secret).delay(&targets[index].name, 5000)
                    };
                    let _ = tx.send((targets[index].id.clone(), latency));
                }
            });
        }
        drop(tx);
        for (id, latency) in rx {
            with_store(shared, |store| {
                store.latencies.insert(id.clone(), latency);
                store.testing_ids.remove(&id);
                if track_progress && store.is_testing_latency {
                    store.latency_completed += 1;
                }
            });
        }
    });
}

// ---------------------------------------------------------------- 监控

pub fn start_monitor(shared: &Shared) {
    let ctx = with_store(shared, |store| store.egui_ctx.clone()).unwrap_or_default();
    let flag = Arc::new(AtomicBool::new(false));
    {
        let mut store = shared.lock().unwrap();
        if let Some(old) = store.monitor_stop.take() {
            old.store(true, Ordering::SeqCst);
        }
        store.monitor_stop = Some(Arc::clone(&flag));
    }
    let shared = Arc::clone(shared);
    let ctx = ctx.clone();
    std::thread::spawn(move || loop {
        let (running, busy, testing, interval) = with_store(&shared, |store| {
            (
                store.running,
                store.busy,
                store.is_testing_latency,
                store.preferences.interval.max(1),
            )
        });
        if !running || flag.load(Ordering::SeqCst) {
            break;
        }
        let seconds = interval;
        let mut slept = 0u64;
        while slept < seconds * 1000 {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
            slept += 200;
        }
        if flag.load(Ordering::SeqCst) {
            break;
        }
        if busy || testing {
            continue;
        }
        let _ = do_inspect(&shared, false);
        ctx.request_repaint();
    });
}

pub fn inspect_now(shared: &Shared, ctx: &egui::Context) {
    perform(shared, ctx, |store, _ctx| {
        if let Err(error) = do_inspect(store, true) {
            fail(store, error);
        }
    });
}

fn do_inspect(shared: &Shared, force: bool) -> Result<(), String> {
    let (running, selected, dynamic, automatic, active_mode) =
        with_store(shared, |store| {
            (
                store.running,
                store.selected.clone(),
                store.dynamic_mode,
                store.preferences.automatic,
                store.active_mode,
            )
        });
    if !running {
        return Ok(());
    }
    // 直连模式没有代理节点可切，只确认内核存活与本地端口出网。
    if !with_store(shared, |s| s.preferences.traffic_mode).uses_proxy() {
        let client = client_of(shared);
        if client.version().is_err() {
            log(shared, "内核失联，正在恢复系统网络");
            do_stop(shared)?;
            return Ok(());
        }
        let internet = client.internet_probe(proxy_port());
        let text = if internet.health == crate::logic::model::Health::Healthy {
            format!("直连模式出网正常 · {} ms", internet.milliseconds)
        } else {
            "直连模式出网失败，请检查本机网络".into()
        };
        with_store(shared, |store| {
            store.internet_status = text;
            store.routing_status = if store.tun || store.system_proxy {
                "直连模式已开启".into()
            } else {
                "直连内核运行中".into()
            };
            store.openai_status = "直连模式不检测代理节点".into();
            store.failures = 0;
        });
        return Ok(());
    }
    let current = with_store(shared, |store| {
        store.route_nodes().into_iter().find(|n| n.name == selected)
    });
    let Some(current) = current else {
        // 当前节点已从订阅或计划中消失：不能静默返回，否则界面会停在旧状态。
        with_store(shared, |store| {
            let text = "当前线路已失效，请重新选择节点或重新连接";
            if store.routing_status != text {
                store.routing_status = text.into();
                store.internet_status = "当前线路不在可用列表中".into();
                store.log(text);
            }
        });
        return Ok(());
    };
    let client = client_of(shared);

    if client.version().is_err() {
        log(shared, "内核失联，正在恢复系统网络");
        do_stop(shared)?;
        return Ok(());
    }
    if dynamic && with_store(shared, |s| s.routing_status != "已打通") {
        return do_rebuild_routes(shared);
    }

    let internet = client.internet_probe(proxy_port());
    let network_text = if internet.health == crate::logic::model::Health::Healthy {
        "HTTPS 出网正常"
    } else {
        "当前线路出网失败，正在检查备用"
    };
    if with_store(shared, |s| s.internet_status != network_text) {
        with_store(shared, |store| store.internet_status = network_text.into());
    }

    // 出网正常不代表 AI 线路可用：AI 模式下 OpenAI 明确失败（地区限制 / 离线）
    // 需要立即切换，不等待连续失败阈值。AI 模式缩短复检周期，普通模式只做质量提示。
    let openai_interval = if active_mode == Some(RoutingMode::Ai) {
        10_000
    } else {
        30_000
    };
    let last_openai = with_store(shared, |s| s.last_openai_probe);
    let mut openai_failed = false;
    if internet.health == crate::logic::model::Health::Healthy
        && now_ms().saturating_sub(last_openai) >= openai_interval
    {
        let ai = client.openai_probe(proxy_port());
        with_store(shared, |store| store.last_openai_probe = now_ms());
        let text = format!("OpenAI：{}", ai.detail);
        with_store(shared, |store| store.openai_status = text);
        openai_failed = active_mode == Some(RoutingMode::Ai)
            && matches!(
                ai.health,
                crate::logic::model::Health::Blocked | crate::logic::model::Health::Offline
            );
    } else if internet.health != crate::logic::model::Health::Healthy {
        with_store(shared, |store| {
            store.openai_status = "OpenAI：等待网络恢复".into()
        });
    }
    let health = internet.clone();

    with_store(shared, |store| store.set_status(&current.id, health.clone()));
    if health.health == crate::logic::model::Health::Healthy {
        with_store(shared, |store| {
            store.failures = 0;
            if store.dynamic_mode && store.routing_status != "已打通" {
                store.routing_status = "已打通".into();
            }
        });
        if openai_failed {
            return do_openai_failover(shared, force);
        }
        return Ok(());
    }
    if matches!(
        health.health,
        crate::logic::model::Health::Limited | crate::logic::model::Health::Unknown
    ) {
        with_store(shared, |store| store.failures = 0);
        log(shared, "OpenAI 返回限流或不确定结果，本次保留节点");
        return Ok(());
    }

    with_store(shared, |store| store.failures += 1);
    let failures = with_store(shared, |s| s.failures);
    if dynamic {
        if let Some(backup) = with_store(shared, |store| store.fresh_standbys().into_iter().next()) {
            do_select(shared, &backup)?;
            with_store(shared, |store| store.routing_status = "已打通".into());
            log(shared, "主线路失效，已切换到预热备用线路；已有请求可能需要重试");
            return Ok(());
        }
    }
    if !automatic && !force {
        return Ok(());
    }
    if dynamic && failures >= 2 {
        return do_rebuild_routes(shared);
    }
    let last_switch = with_store(shared, |s| s.last_switch);
    if !force && !HealthPolicy::should_failover(failures, last_switch, now_ms(), 2) {
        return Ok(());
    }
    if dynamic {
        return do_rebuild_routes(shared);
    }

    let alternatives: Vec<ProxyNode> = with_store(shared, |store| {
        store
            .route_nodes()
            .into_iter()
            .filter(|n| n.id != current.id)
            .collect()
    });
    if alternatives.is_empty() {
        log(shared, "没有备用节点");
        return Ok(());
    }
    let offset = with_store(shared, |s| s.scan_offset) % alternatives.len();
    let rotated: Vec<ProxyNode> = alternatives[offset..]
        .iter()
        .chain(alternatives[..offset].iter())
        .cloned()
        .collect();
    with_store(shared, |store| {
        store.scan_offset = (offset + 12) % alternatives.len();
    });
    let candidates: Vec<ProxyNode> = rotated
        .into_iter()
        .filter(|node| {
            with_store(shared, |store| match store.statuses.get(&node.id) {
                None => true,
                Some(status) => {
                    status.result.health == crate::logic::model::Health::Healthy
                        || now_ms().saturating_sub(status.checked) > 60_000
                }
            })
        })
        .take(12)
        .collect();

    let mut best: Option<(ProxyNode, ProbeResult)> = None;
    for node in candidates {
        let result = do_check(shared, &node, active_mode == Some(RoutingMode::Ai))?;
        if result.health == crate::logic::model::Health::Healthy
            && best
                .as_ref()
                .map(|(_, b)| result.milliseconds < b.milliseconds)
                .unwrap_or(true)
        {
            best = Some((node, result));
        }
        if let Some((_, ref result)) = best {
            if result.milliseconds < 1500 {
                break;
            }
        }
    }
    match best {
        Some((node, result)) => {
            do_select(shared, &node)?;
            log(
                shared,
                &format!(
                    "OpenAI 连通性恢复：{} ms；已有连接可能需要重试",
                    result.milliseconds
                ),
            );
            Ok(())
        }
        None => {
            log(shared, "本批备用节点均未确认可用，保留当前节点，稍后重试");
            Ok(())
        }
    }
}

/// AI 模式下 OpenAI 明确不可达时的快速切换：跳过连续失败阈值，立即用 PROBE 组
/// 复检备用线路并切到最先确认可达的节点。30 秒内只允许一次，避免线路来回抖动。
fn do_openai_failover(shared: &Shared, force: bool) -> Result<(), String> {
    let (automatic, selected, latencies, candidates, measurements) = with_store(shared, |store| {
        (
            store.preferences.automatic,
            store.selected.clone(),
            store.latencies.clone(),
            store.route_nodes(),
            store.measurements.clone(),
        )
    });
    if !automatic && !force {
        log(shared, "OpenAI 不可达，但自动切换已关闭，保留当前线路");
        return Ok(());
    }
    let now = now_ms();
    let last = with_store(shared, |s| s.last_openai_failover);
    if now.saturating_sub(last) < 30_000 {
        log(shared, "OpenAI 探测失败，距上次 AI 线路切换不足 30 秒，等待下一轮复检");
        return Ok(());
    }

    // 先记录当前线路的 OpenAI 结果，避免后续仍按“已验证”优先选回它。
    // measurements 以节点 id 为键，不能用 selected 里的节点名称。
    let current_id = candidates
        .iter()
        .find(|node| node.name == selected)
        .map(|node| node.id.clone());
    with_store(shared, |store| {
        if let Some(id) = &current_id {
            let mut measured = store.measurements.get(id).cloned().unwrap_or_default();
            measured.openai_reachable = Some(false);
            measured.openai_checked = Some(now);
            store.measurements.insert(id.clone(), measured);
        }
        store.last_openai_failover = now;
        store.openai_status = "OpenAI：不可达，正在切换线路".into();
        store.save_measurements();
    });

    let verified = |node: &ProxyNode| {
        measurements
            .get(&node.id)
            .and_then(|m| {
                (m.openai_reachable == Some(true))
                    .then(|| m.openai_checked)
                    .flatten()
                    .map(|checked| now.saturating_sub(checked) <= 15 * 60_000)
            })
            .unwrap_or(false)
    };
    let cooling = |node: &ProxyNode| {
        measurements
            .get(&node.id)
            .and_then(|m| {
                (m.openai_reachable == Some(false))
                    .then(|| m.openai_checked)
                    .flatten()
                    .map(|checked| now.saturating_sub(checked) <= 60_000)
            })
            .unwrap_or(false)
    };

    let mut alternatives: Vec<ProxyNode> = candidates
        .into_iter()
        .filter(|node| node.name != selected && !cooling(node))
        .collect();
    alternatives = NodeRanking::sorted(
        &alternatives,
        &latencies,
        crate::logic::model::NodeSortOrder::Latency,
    );
    // 稳定排序：最近确认过 OpenAI 可达的线路排在最前面，其余按延迟顺序复检。
    alternatives.sort_by_key(|node| !verified(node));
    alternatives.truncate(8);

    if alternatives.is_empty() {
        log(shared, "OpenAI 探测失败，但没有可复检的备用线路，保留当前线路");
        with_store(shared, |store| {
            store.openai_status = "OpenAI：不可达，暂无可用备用线路".into();
        });
        return Ok(());
    }

    let deadline = now_ms() + 30_000;
    for node in alternatives {
        if now_ms() >= deadline {
            break;
        }
        match do_check(shared, &node, true) {
            Ok(result) if result.health == crate::logic::model::Health::Healthy => {
                do_select(shared, &node)?;
                with_store(shared, |store| {
                    store.openai_status = format!(
                        "OpenAI 探测失败，已切换至 {} · {} ms",
                        node.name, result.milliseconds
                    );
                    // 新线路立刻参加下一轮 OpenAI 复检。
                    store.last_openai_probe = 0;
                    store.save_measurements();
                });
                log(
                    shared,
                    &format!("OpenAI 探测失败，已立即切换至 {}；已有请求可能需要重试", node.name),
                );
                return Ok(());
            }
            Ok(_) => {}
            Err(error) => log(shared, &format!("备用线路 {} 复检失败：{error}", node.name)),
        }
    }
    with_store(shared, |store| {
        store.openai_status = "OpenAI：不可达，备用线路均未通过复检".into();
        store.save_measurements();
    });
    log(shared, "OpenAI 探测失败，备用线路均未通过复检，保留当前线路等待下一轮");
    Ok(())
}

fn do_rebuild_routes(shared: &Shared) -> Result<(), String> {
    let (nodes, plan, secret, selected, tun, explore, round, traffic_mode, kind) =
        with_store(shared, |store| {
            (
                store.nodes.clone(),
                store.route_plan.clone(),
                store.preferences.secret.clone(),
                store.selected.clone(),
                store.tun,
                store.preferences.explore_multihop == Some(true),
                store.search_round,
                store.preferences.traffic_mode,
                store.preferences.kernel,
            )
        });
    let latencies = with_store(shared, |s| s.latencies.clone());
    let ordered = NodeRanking::connection_candidates(&nodes, &latencies, now_ms());
    let next = DynamicRouter::plan(&ordered, round + 1, 12, explore)?;

    let mut candidates = Vec::new();
    if let Some(current) = plan.candidates.iter().find(|n| n.name == selected) {
        candidates.push(current.clone());
    }
    for node in &next.candidates {
        if !candidates.iter().any(|n| n.name == node.name) {
            candidates.push(node.clone());
        }
    }
    let mut definitions = plan.definitions.clone();
    for node in &next.definitions {
        if !definitions.iter().any(|n| n.id == node.id) {
            definitions.push(node.clone());
        }
    }
    let built = configuration::make_for(
        kind,
        &nodes,
        &secret,
        tun,
        Some(&candidates),
        &definitions,
        None,
        Some(&selected),
        traffic_mode,
    )?;
    for warning in &built.warnings {
        log(shared, warning);
    }
    if kind == KernelKind::Mihomo {
        CoreClient::new(kind, &secret).reload_config(&String::from_utf8_lossy(&built.data))?;
        with_store(shared, |store| {
            store.search_round = round + 1;
            store.route_plan = RoutePlan {
                candidates,
                definitions,
            };
            store.log("已下发下一批候选链路");
            store.publish_route();
        });
    } else {
        with_store(shared, |store| {
            store.search_round = round + 1;
            store.route_plan = RoutePlan {
                candidates,
                definitions,
            };
            store.log("已下发下一批候选链路（重启内核生效）");
            store.publish_route();
        });
        restart_core(shared, Some(selected), true)?;
    }
    do_ensure_route(shared, true, None)
}

// ---------------------------------------------------------------- 流量

fn start_traffic(shared: &Shared) {
    let (secret, kind, data) = with_store(shared, |store| {
        (
            store.preferences.secret.clone(),
            store.preferences.kernel,
            Arc::clone(&store.traffic),
        )
    });
    if let Some(flag) = with_store(shared, |store| store.traffic_stop.take()) {
        flag.store(true, Ordering::SeqCst);
    }
    let flag = Arc::new(AtomicBool::new(false));
    with_store(shared, |store| store.traffic_stop = Some(Arc::clone(&flag)));
    traffic::start(
        CoreClient::new(kind, &secret),
        data,
        flag,
        crate::logic::model::random_hex(8),
    );
}

/// 启动实时连接轮询与 IP 定位（地球面板数据源）。
fn start_activity(shared: &Shared) {
    let (secret, kind, data) = with_store(shared, |store| {
        (
            store.preferences.secret.clone(),
            store.preferences.kernel,
            Arc::clone(&store.activity),
        )
    });
    if let Some(flag) = with_store(shared, |store| store.activity_stop.take()) {
        flag.store(true, Ordering::SeqCst);
    }
    let flag = Arc::new(AtomicBool::new(false));
    with_store(shared, |store| store.activity_stop = Some(Arc::clone(&flag)));
    activity::start(shared, CoreClient::new(kind, &secret), data, flag);
}

/// 切换代理内核。运行中会自动用新内核重启；Xray 不支持 TUN，需先关闭。
pub fn change_kernel(shared: &Shared, ctx: &egui::Context, kind: KernelKind) {
    perform(shared, ctx, move |store, _ctx| {
        let (current, running, tun, recovery) = with_store(store, |s| {
            (s.preferences.kernel, s.running, s.tun, s.recovery)
        });
        if current == kind {
            return;
        }
        if recovery {
            fail(store, "请先恢复上次网络设置，再切换内核");
            return;
        }
        if tun && !kind.capabilities().tun {
            fail(store, "请先关闭 TUN，再切换到不支持 TUN 的内核");
            return;
        }
        with_store(store, |s| {
            s.preferences.kernel = kind;
            s.core.set_kind(kind);
            s.save_preferences();
            s.rule_status = match kind.capabilities().rules {
                RulesSupport::Provider => "尚未手动更新，内核运行后每小时自动刷新".into(),
                RulesSupport::Restart => "由内核自动刷新，手动更新会重启内核".into(),
                RulesSupport::Builtin => "使用内置 geo 数据，不支持在线更新".into(),
            };
            s.log(&format!("切换内核：{}", kind.title()));
        });
        if running {
            if let Err(error) = restart_core(store, None, true) {
                // 新内核启动失败时回滚到原内核，避免界面与已停止的内核状态不一致。
                with_store(store, |s| {
                    s.preferences.kernel = current;
                    s.core.set_kind(current);
                    s.save_preferences();
                });
                let restored = restart_core(store, None, true).is_ok();
                with_store(store, |s| {
                    if !restored {
                        s.recovery = true;
                        s.log("内核切换回滚失败，请停止并恢复网络");
                    }
                });
                fail(store, format!("内核切换失败：{error}"));
            }
        }
    });
}

/// 修改本地混合代理端口。运行中会自动重启内核使新端口生效；失败时回滚。
pub fn change_proxy_port(shared: &Shared, ctx: &egui::Context, port: u16) {
    perform(shared, ctx, move |store, _ctx| {
        let current = with_store(store, |s| s.preferences.proxy_port);
        if current == port {
            return;
        }
        if let Err(message) = crate::logic::model::validate_proxy_port(port) {
            fail(store, message);
            return;
        }
        let running = with_store(store, |s| s.running);
        with_store(store, |s| {
            s.preferences.proxy_port = port;
            crate::logic::model::set_proxy_port(port);
            s.save_preferences();
            s.log(&format!("本地代理端口改为 {port}"));
        });
        if running {
            if let Err(error) = restart_core(store, None, true) {
                // 新端口启动失败时回滚到原端口，避免界面与已停止的内核状态不一致。
                with_store(store, |s| {
                    s.preferences.proxy_port = current;
                    crate::logic::model::set_proxy_port(current);
                    s.save_preferences();
                    s.log(&format!("端口 {port} 启动失败，已回滚为 {current}"));
                });
                let restored = restart_core(store, None, true).is_ok();
                with_store(store, |s| {
                    if !restored {
                        s.recovery = true;
                        s.log("端口修改回滚失败，请停止并恢复网络");
                    }
                });
                fail(store, format!("使用端口 {port} 重启内核失败：{error}"));
            } else {
                with_store(store, |s| s.log("内核已用新端口重启"));
            }
        }
    });
}

pub fn shutdown(shared: &Shared) {
    with_store(shared, |s| s.epoch = s.epoch.wrapping_add(1));
    let _ = do_stop(shared);
}
