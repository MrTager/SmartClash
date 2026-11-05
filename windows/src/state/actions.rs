use crate::logic::configuration;
use crate::logic::dynamic_router::{DynamicRouter, RoutePlan};
use crate::logic::health::{now_ms, HealthPolicy};
use crate::logic::model::{
    NodeLatency, ProbeResult, ProxyNode, RoutingMode, TrafficMode, PROBE_PORT, PROXY_PORT,
};
use crate::logic::ranking::{ModeRanking, NodeRanking};
use crate::services::core_client::CoreClient;
use crate::services::network;
use crate::services::paths;
use crate::services::subscription_service;
use crate::services::system_proxy::SystemProxy;
use crate::state::activity;
use crate::state::traffic;
use crate::state::{with_store, Shared};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Connection,
    LatencyTest,
}

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
    let (sources, running, secret) = with_store(shared, |store| {
        (
            store.preferences.sources.clone(),
            store.running,
            store.preferences.secret.clone(),
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

    if running {
        let (plan, selected, traffic_mode) = with_store(shared, |s| {
            (s.route_plan.clone(), s.selected.clone(), s.preferences.traffic_mode)
        });
        let routes = if plan.candidates.is_empty() {
            None
        } else {
            Some(plan.candidates.as_slice())
        };
        let data = configuration::make(
            &update.nodes,
            &secret,
            false,
            routes,
            &plan.definitions,
            None,
            Some(&selected),
            traffic_mode,
        )?;
        let temp = paths::data_root().join("reload.json");
        paths::write_private(&temp, &data)?;
        let exe = paths::core_binary();
        let test = crate::services::command::run(
            &exe,
            &[
                "-t".into(),
                "-d".into(),
                paths::data_root().display().to_string(),
                "-f".into(),
                temp.display().to_string(),
            ],
        )
        .map_err(|e| format!("无法校验更新配置：{e}"))?;
        if test.code != 0 {
            return Err("更新配置未通过内核校验，继续使用旧配置".into());
        }
        let client = CoreClient::new(&secret);
        client.reload_config_path(&temp.display().to_string())?;
        let available: Vec<ProxyNode> = if plan.candidates.is_empty() {
            update.nodes.clone()
        } else {
            plan.candidates.clone()
        };
        let next = available
            .iter()
            .find(|n| n.name == selected)
            .or_else(|| available.first())
            .map(|n| n.name.clone())
            .unwrap_or_default();
        if !next.is_empty() {
            let _ = client.select(&next, "SMART");
        }
    }

    with_store(shared, |store| {
        store.nodes = update.nodes.clone();
        let _ = store.aggregate.update(&store.nodes.clone());
        store.last_update = now_ms();
        store.log(&format!("订阅已合并：{} 个去重节点", store.nodes.len()));
        for warning in &update.warnings {
            store.log(warning);
        }
    });
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
    let (nodes, secret, plan, latencies, traffic_mode) = with_store(shared, |store| {
        (
            store.nodes.clone(),
            store.preferences.secret.clone(),
            store.route_plan.clone(),
            store.latencies.clone(),
            store.preferences.traffic_mode,
        )
    });
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
    let data = configuration::make(
        &nodes,
        &secret,
        tun,
        routes,
        &plan.definitions,
        outbound.as_deref(),
        preferred_name.as_deref(),
        traffic_mode,
    )?;

    let mut core = with_store(shared, |store| std::mem::take(&mut store.core));
    let client = CoreClient::new(&secret);
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
                route_nodes
                    .iter()
                    .find(|n| Some(&n.name) == preferred_name.as_ref())
                    .or_else(|| route_nodes.first())
                    .map(|n| n.name.clone())
                    .unwrap_or_default()
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
                store.publish_route();
                store.log(match purpose {
                    Purpose::LatencyTest => "仅测速内核已启动，未接管系统网络",
                    Purpose::Connection if !traffic_mode.uses_proxy() => "直连模式内核已启动",
                    Purpose::Connection if tun => "TUN 已启动",
                    Purpose::Connection => "代理内核已启动",
                });
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

    // 必须先恢复系统代理，再移除它指向的本地端点。
    let restore = SystemProxy::restore();
    if let Err(error) = restore {
        with_store(shared, |store| {
            store.recovery = true;
            store.system_proxy = SystemProxy::has_backup();
        });
        return Err(error);
    }

    let mut core = with_store(shared, |store| std::mem::take(&mut store.core));
    let result = core.stop();
    let recovery = core.recovery_needed();
    with_store(shared, |store| store.core = core);

    with_store(shared, |store| {
        store.system_proxy = false;
        store.running = false;
        store.tun = false;
        store.selected.clear();
        store.dynamic_mode = false;
        store.route_plan = RoutePlan::default();
        store.routing_status = "尚未打通".into();
        store.internet_status = "未连接".into();
        store.active_mode = None;
        store.recovery = recovery;
        store.publish_route();
        store.log("代理已停止，网络设置已恢复");
    });

    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            with_store(shared, |store| store.recovery = true);
            Err(error)
        }
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
            store.dynamic_mode = false;
            store.routing_status = "系统代理已关闭，内核继续运行".into();
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
    let result = do_ensure_route(shared, true, Some(epoch)).and_then(|_| SystemProxy::enable());
    match result {
        Ok(()) => {
            with_store(shared, |store| {
                store.system_proxy = true;
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

pub fn toggle_tun(shared: &Shared, ctx: &egui::Context) {
    perform(shared, ctx, |store, _ctx| {
        if let Err(error) = do_toggle_tun(store) {
            fail(store, error);
        }
    });
}

fn do_toggle_tun(shared: &Shared) -> Result<(), String> {
    let epoch = with_store(shared, |s| s.epoch);
    let (tun, dynamic, previous, plan) = with_store(shared, |store| {
        (
            store.tun,
            store.dynamic_mode,
            store.selected.clone(),
            store.route_plan.clone(),
        )
    });
    if tun {
        do_stop(shared)?;
        if cancelled(shared, epoch) {
            return Err("操作已停止".into());
        }
        with_store(shared, |store| {
            store.dynamic_mode = dynamic;
            store.route_plan = plan;
        });
        return do_start(shared, false, Some(previous), Purpose::Connection);
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
    do_stop(shared)?;
    if cancelled(shared, epoch) {
        return Err("操作已停止".into());
    }
    with_store(shared, |store| {
        store.dynamic_mode = dynamic;
        store.route_plan = plan;
    });
    do_start(shared, true, Some(verified), Purpose::Connection)?;

    let secret = with_store(shared, |s| s.preferences.secret.clone());
    let client = CoreClient::new(&secret);
    let result = client.internet_probe(PROXY_PORT);
    if result.health != crate::logic::model::Health::Healthy {
        let _ = do_stop(shared);
        return Err("TUN 接管后出网验证失败，已尝试自动恢复网络；请检查 DNS、物理网卡或其他 VPN 冲突".into());
    }
    with_store(shared, |store| {
        store.internet_status = format!("TUN 出网已验证 · {} ms", result.milliseconds);
        store.routing_status = "已打通".into();
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
        store.preferences.automatic = true;
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
    let (current, running, recovery, node_count, secret) = with_store(shared, |store| {
        (
            store.preferences.traffic_mode,
            store.running,
            store.recovery,
            store.nodes.len(),
            store.preferences.secret.clone(),
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
    let client = CoreClient::new(&secret);
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
        let secret = with_store(shared, |store| store.preferences.secret.clone());
        let client = CoreClient::new(&secret);
        let result = client.internet_probe(PROXY_PORT);
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
    let mut latencies;
    let (nodes, measurements) = with_store(shared, |store| {
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

    let client = CoreClient::new(&secret);
    let deadline = now_ms() + 45_000;
    let mut tested: HashSet<String> = HashSet::new();
    let mut speed_attempts = 0usize;

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

            // AI 模式优先使用 OpenAI 已验证节点；一个都没验证过时仍按普通出网连接，
            // 由后台监控继续确认 OpenAI 可达性，避免一键连接被单点验证拖死。
            if mode == RoutingMode::Ai {
                reachable.sort_by_key(|node| {
                    !measurements
                        .get(&node.id)
                        .and_then(|m| m.openai_reachable)
                        .unwrap_or(false)
                });
            }

            let mut eligible = reachable.clone();
            if mode == RoutingMode::HighSpeed {
                with_store(shared, |store| {
                    store.routing_status = "正在抽测下载吞吐（每轮约 1 MiB）".into();
                });
                for node in &reachable {
                    if speed_attempts >= 3 || now_ms() >= deadline {
                        break;
                    }
                    speed_attempts += 1;
                    let sample = with_probe(shared, node, |_| {
                        let client = CoreClient::new(&secret);
                        client.throughput(PROBE_PORT)
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
                let result = client.internet_probe(PROXY_PORT);
                if result.health == crate::logic::model::Health::Healthy {
                    with_store(shared, |store| {
                        store.internet_status =
                            format!("HTTPS 出网已验证 · {} ms", result.milliseconds);
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
    let (running, plan, secret, selected, tun, traffic_mode) = with_store(shared, |store| {
        (
            store.running,
            store.route_plan.clone(),
            store.preferences.secret.clone(),
            store.selected.clone(),
            store.tun,
            store.preferences.traffic_mode,
        )
    });
    if !running {
        return Err("请先启动内核".into());
    }
    if !traffic_mode.uses_proxy() {
        return Err("当前是直连模式，不切换节点；请先切换到规则或全局代理".into());
    }
    let client = CoreClient::new(&secret);
    if !plan.candidates.is_empty() && !plan.candidates.iter().any(|n| n.id == node.id) {
        let (nodes, definitions) = with_store(shared, |store| {
            (store.nodes.clone(), store.route_plan.definitions.clone())
        });
        let mut candidates = plan.candidates.clone();
        candidates.push(node.clone());
        let data = configuration::make(
            &nodes,
            &secret,
            tun,
            Some(&candidates),
            &definitions,
            None,
            Some(&selected),
            traffic_mode,
        )?;
        let text = String::from_utf8_lossy(&data).to_string();
        client.reload_config(&text)?;
        with_store(shared, |store| {
            store.route_plan = RoutePlan {
                candidates,
                definitions,
            };
        });
    }
    client.select(&node.name, "SMART")?;
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
    operation: impl FnOnce(&ProxyNode) -> T,
) -> Result<T, String> {
    let secret = with_store(shared, |s| s.preferences.secret.clone());
    let client = CoreClient::new(&secret);
    client.select(&node.name, "PROBE")?;
    Ok(operation(node))
}

fn do_check(shared: &Shared, node: &ProxyNode, openai: bool) -> Result<ProbeResult, String> {
    let secret = with_store(shared, |s| s.preferences.secret.clone());
    let result = with_probe(shared, node, |_| {
        let client = CoreClient::new(&secret);
        if openai {
            client.openai_probe(PROBE_PORT)
        } else {
            client.internet_probe(PROBE_PORT)
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
    let (running, secret) = with_store(shared, |s| (s.running, s.preferences.secret.clone()));
    if !running {
        return Err("请先启动内核或运行一次延迟测试".into());
    }
    if !with_store(shared, |s| s.preferences.traffic_mode).uses_proxy() {
        return Err("当前是直连模式，不切换节点；请先切换到规则或全局代理".into());
    }
    let client = CoreClient::new(&secret);
    let previous = with_store(shared, |store| {
        store
            .route_nodes()
            .into_iter()
            .find(|n| n.name == store.selected)
    });
    let sample = client.delay(&node.name, 5000);
    with_store(shared, |store| {
        store.latencies.insert(node.id.clone(), sample.clone());
        store.save_latencies();
    });
    if !sample.reachable() {
        return Err("该节点本次 HTTPS 测试未通过，已保留原业务出口".into());
    }
    do_select(shared, &node)?;
    let result = client.internet_probe(PROXY_PORT);
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
    let targets = with_store(shared, |s| s.nodes.clone());
    begin_latency(shared, ctx, targets);
}

pub fn begin_latency_one(shared: &Shared, ctx: &egui::Context, node: ProxyNode) {
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
    let secret = with_store(shared, |s| s.preferences.secret.clone());
    let next = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel::<(String, NodeLatency)>();
    let concurrency = 6.min(targets.len());

    std::thread::scope(|scope| {
        for _ in 0..concurrency {
            let tx = tx.clone();
            let secret = secret.clone();
            let cancel = Arc::clone(cancel);
            let next = Arc::clone(&next);
            scope.spawn(move || {
                let client = CoreClient::new(&secret);
                loop {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    if index >= targets.len() {
                        break;
                    }
                    let latency = client.delay(&targets[index].name, 5000);
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
        let (running, busy, testing, interval, dynamic) = with_store(&shared, |store| {
            (
                store.running,
                store.busy,
                store.is_testing_latency,
                store.preferences.interval.max(1),
                store.dynamic_mode,
            )
        });
        if !running || flag.load(Ordering::SeqCst) {
            break;
        }
        let seconds = if dynamic { 1 } else { interval.min(60) };
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
    let (running, selected, dynamic, automatic, secret, active_mode) =
        with_store(shared, |store| {
            (
                store.running,
                store.selected.clone(),
                store.dynamic_mode,
                store.preferences.automatic,
                store.preferences.secret.clone(),
                store.active_mode,
            )
        });
    if !running {
        return Ok(());
    }
    // 直连模式没有代理节点可切，只确认内核存活与本地端口出网。
    if !with_store(shared, |s| s.preferences.traffic_mode).uses_proxy() {
        let client = CoreClient::new(&secret);
        if client.version().is_err() {
            log(shared, "内核失联，正在恢复系统网络");
            do_stop(shared)?;
            return Ok(());
        }
        let internet = client.internet_probe(PROXY_PORT);
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
        return Ok(());
    };
    let client = CoreClient::new(&secret);

    if client.version().is_err() {
        log(shared, "内核失联，正在恢复系统网络");
        do_stop(shared)?;
        return Ok(());
    }
    if dynamic && with_store(shared, |s| s.routing_status != "已打通") {
        return do_rebuild_routes(shared);
    }

    let internet = client.internet_probe(PROXY_PORT);
    let network_text = if internet.health == crate::logic::model::Health::Healthy {
        "HTTPS 出网正常"
    } else {
        "当前线路出网失败，正在检查备用"
    };
    if with_store(shared, |s| s.internet_status != network_text) {
        with_store(shared, |store| store.internet_status = network_text.into());
    }

    // OpenAI 可达性只作为质量提示，不参与线路存活判定：能正常出网就先保持连接，
    // 否则 AI 模式下会反复切换节点，表现为节点和状态一直在闪。
    let last_openai = with_store(shared, |s| s.last_openai_probe);
    if internet.health == crate::logic::model::Health::Healthy
        && now_ms().saturating_sub(last_openai) >= 30_000
    {
        let ai = client.openai_probe(PROXY_PORT);
        with_store(shared, |store| store.last_openai_probe = now_ms());
        let text = format!("OpenAI：{}", ai.detail);
        with_store(shared, |store| store.openai_status = text);
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
        if with_store(shared, |s| s.dynamic_mode && !s.system_proxy && !s.tun) {
            if SystemProxy::enable().is_ok() {
                with_store(shared, |store| store.system_proxy = true);
            }
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

fn do_rebuild_routes(shared: &Shared) -> Result<(), String> {
    let (nodes, plan, secret, selected, tun, explore, round, traffic_mode) =
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
    let data = configuration::make(
        &nodes,
        &secret,
        tun,
        Some(&candidates),
        &definitions,
        None,
        Some(&selected),
        traffic_mode,
    )?;
    let client = CoreClient::new(&secret);
    client.reload_config(&String::from_utf8_lossy(&data))?;
    with_store(shared, |store| {
        store.search_round = round + 1;
        store.route_plan = RoutePlan {
            candidates,
            definitions,
        };
        store.log("已下发下一批候选链路");
        store.publish_route();
    });
    do_ensure_route(shared, true, None)
}

// ---------------------------------------------------------------- 流量

fn start_traffic(shared: &Shared) {
    let (secret, data) = with_store(shared, |store| {
        (store.preferences.secret.clone(), Arc::clone(&store.traffic))
    });
    if let Some(flag) = with_store(shared, |store| store.traffic_stop.take()) {
        flag.store(true, Ordering::SeqCst);
    }
    let flag = Arc::new(AtomicBool::new(false));
    with_store(shared, |store| store.traffic_stop = Some(Arc::clone(&flag)));
    traffic::start(
        CoreClient::new(&secret),
        data,
        flag,
        crate::logic::model::random_hex(8),
    );
}

/// 启动实时连接轮询与 IP 定位（地球面板数据源）。
fn start_activity(shared: &Shared) {
    let (secret, data) = with_store(shared, |store| {
        (
            store.preferences.secret.clone(),
            Arc::clone(&store.activity),
        )
    });
    if let Some(flag) = with_store(shared, |store| store.activity_stop.take()) {
        flag.store(true, Ordering::SeqCst);
    }
    let flag = Arc::new(AtomicBool::new(false));
    with_store(shared, |store| store.activity_stop = Some(Arc::clone(&flag)));
    activity::start(shared, CoreClient::new(&secret), data, flag);
}

pub fn shutdown(shared: &Shared) {
    with_store(shared, |s| s.epoch = s.epoch.wrapping_add(1));
    let _ = do_stop(shared);
}

pub fn core_executable() -> std::path::PathBuf {
    paths::core_binary()
}
