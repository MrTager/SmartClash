use super::widgets::{self, hint};
use super::{Action, ViewState};
use crate::state::Store;
use egui::{RichText, Ui};

pub fn draw(view: &mut ViewState, store: &mut Store, ui: &mut Ui, pending: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("连接控制")
                .size(18.0)
                .strong()
                .color(widgets::TEXT),
        );
        let (status, color) = if store.stopping {
            ("正在停止并恢复网络", widgets::WARN)
        } else if store.recovery {
            ("网络设置待恢复", widgets::WARN)
        } else if store.busy {
            ("正在执行操作", widgets::ACCENT)
        } else if store.is_testing_latency {
            ("正在测速", widgets::ACCENT)
        } else if store.running {
            ("内核运行中", widgets::GOOD)
        } else {
            ("内核已停止", widgets::MUTED)
        };
        ui.label(RichText::new(status).size(12.0).color(color));
        if store.running || store.busy || store.recovery || store.system_proxy || store.tun {
            if ui
                .add_enabled(
                    !store.stopping,
                    egui::Button::new(
                        RichText::new(if store.recovery {
                            "恢复网络"
                        } else {
                            "停止并恢复网络"
                        })
                        .size(12.0)
                        .color(widgets::WARN),
                    )
                    .fill(widgets::WARN_DIM)
                    .stroke(egui::Stroke::new(1.0_f32, widgets::WARN)),
                )
                .clicked()
            {
                pending.push(Action::Stop);
            }
        }
    });
    ui.add_space(6.0);

    core_controls(store, ui, pending);
    ui.add_space(6.0);

    if store.running && !store.stopping && !store.recovery && !store.is_testing_latency {
        ui.horizontal_wrapped(|ui| {
            let (fg, bg) = widgets::status_color(&store.internet_status);
            widgets::status_pill(ui, &store.internet_status, fg, bg);
            let (fg, bg) = widgets::status_color(&store.openai_status);
            widgets::status_pill(ui, &store.openai_status, fg, bg);
        });
        ui.add_space(6.0);
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        widgets::metric(
            ui,
            "聚合节点",
            &store.nodes.len().to_string(),
            widgets::ACCENT,
        );
        widgets::metric(
            ui,
            "内核状态",
            if store.stopping {
                "正在停止"
            } else if store.running {
                "运行中"
            } else {
                "已停止"
            },
            if store.running && !store.stopping {
                widgets::GOOD
            } else {
                widgets::MUTED
            },
        );
        let exit = if store.stopping {
            "正在停止"
        } else if store.recovery {
            "待恢复网络"
        } else if !store.running {
            "尚未连接"
        } else if !store.preferences.traffic_mode.uses_proxy() {
            "直连"
        } else if store.selected.is_empty() {
            "尚未选路"
        } else {
            store.selected.as_str()
        };
        widgets::metric(ui, "当前出口", exit, widgets::TEXT);
    });
    ui.add_space(6.0);
    super::globe::draw(store, ui, view);
    ui.add_space(4.0);

    egui::CollapsingHeader::new(RichText::new("高级设置").strong().color(widgets::TEXT))
        .id_salt("overview_advanced")
        .default_open(false)
        .show(ui, |ui| {
            widgets::kernel_controls(store, ui, pending);
            widgets::traffic_mode_details(store, ui, pending);
            widgets::routing_details(store, ui);
            widgets::card(ui, Some("内核与端口"), |ui| {
                ui.horizontal_wrapped(|ui| {
                    let can_start = !store.running && widgets::controls_enabled(store);
                    if widgets::accent_button(ui, "连接并验证", can_start) {
                        pending.push(Action::Connect);
                    }
                    if ui
                        .add_enabled(can_start, egui::Button::new("仅启动内核"))
                        .clicked()
                    {
                        pending.push(Action::StartOnly);
                    }
                    hint(
                        ui,
                        &format!(
                            "HTTP / SOCKS · 127.0.0.1:{}",
                            crate::logic::model::proxy_port()
                        ),
                    );
                });
                hint(ui, "连接并验证会接管系统代理；仅启动内核不接管系统网络。");
                ui.add_space(8.0);
                port_controls(view, store, ui, pending);
            });
            automatic_controls(store, ui, pending);
        });
}

fn core_controls(store: &mut Store, ui: &mut Ui, pending: &mut Vec<Action>) {
    widgets::card(ui, None, |ui| {
        ui.spacing_mut().item_spacing.x = widgets::CONTROL_GAP;
        let enabled = widgets::controls_enabled(store);
        let tun_supported = store.preferences.kernel.capabilities().tun;
        ui.columns(2, |columns| {
            if widgets::proxy_switch(
                &mut columns[0],
                "系统代理",
                "开启 Windows 系统 HTTP / HTTPS 代理（当前用户）；可与 TUN 同时开启，互不影响。",
                store.system_proxy,
                enabled,
            ).clicked() {
                pending.push(Action::ToggleSystem);
            }
            if widgets::proxy_switch(
                &mut columns[1],
                "TUN 虚拟网卡",
                if tun_supported {
                    "接管更多应用流量。首次需要管理员授权，后续复用计划任务；计划任务不可用或执行失败时会回退旧流程，可能再次请求授权。"
                } else {
                    "当前内核不支持 TUN；请在高级设置切换到 Mihomo 或 sing-box。"
                },
                store.tun,
                enabled && tun_supported,
            ).clicked() {
                pending.push(Action::ToggleTun);
            }
        });
        ui.add_space(4.0);
        ui.separator();
        widgets::traffic_mode_selector(store, ui, pending);
        ui.add_space(4.0);
        ui.separator();
        widgets::routing_selector(store, ui, pending);
    });
}

fn automatic_controls(store: &mut Store, ui: &mut Ui, pending: &mut Vec<Action>) {
    widgets::card(ui, Some("智能切换"), |ui| {
        let mut automatic = store.preferences.automatic;
        if ui
            .checkbox(&mut automatic, "OpenAI 或网络不可用时自动切换节点")
            .changed()
        {
            store.preferences.automatic = automatic;
            store.save_preferences();
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("检测间隔");
            egui::ComboBox::from_id_salt("interval")
                .selected_text(interval_text(store.preferences.interval))
                .show_ui(ui, |ui| {
                    for value in [1u64, 5, 15, 60, 300] {
                        if ui
                            .selectable_label(
                                store.preferences.interval == value,
                                interval_text(value),
                            )
                            .clicked()
                        {
                            store.preferences.interval = value;
                            store.save_preferences();
                        }
                    }
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        store.running && widgets::controls_enabled(store),
                        egui::Button::new("立即检测 / 故障切换"),
                    )
                    .clicked()
                {
                    pending.push(Action::InspectNow);
                }
            });
        });
        ui.add_space(4.0);
        hint(
            ui,
            "连续失败 2 次触发切换，切换冷却 30 秒。限流与不确定结果不触发切换。检测不重叠，实际周期包含请求耗时；推荐 1 分钟。",
        );
        hint(
            ui,
            "未使用 API Key：检测仅确认 OpenAI API 的访问结果，不保证 ChatGPT 网页登录、验证码或账号可用性。切换影响新连接，已有请求可能需要重试。",
        );
    });
}

/// 本地代理端口编辑：缓冲仅在与偏好值不同步时刷新，避免逐帧覆盖正在输入的内容。
fn port_controls(view: &mut ViewState, store: &mut Store, ui: &mut Ui, pending: &mut Vec<Action>) {
    if view.proxy_port_synced_to != store.preferences.proxy_port {
        view.proxy_port_input = store.preferences.proxy_port.to_string();
        view.proxy_port_synced_to = store.preferences.proxy_port;
    }
    ui.horizontal(|ui| {
        ui.label("本地代理端口");
        let input = ui.add(
            egui::TextEdit::singleline(&mut view.proxy_port_input)
                .desired_width(72.0)
                .hint_text(crate::logic::model::DEFAULT_PROXY_PORT.to_string()),
        );
        let enter = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let can_apply = !store.busy && !store.stopping && !store.recovery;
        let mut submit = enter && input.changed();
        if widgets::accent_button(ui, "应用", can_apply) {
            submit = true;
        }
        if submit {
            match view.proxy_port_input.trim().parse::<u16>() {
                Ok(port) => {
                    if port != store.preferences.proxy_port {
                        pending.push(Action::ProxyPort(port));
                    }
                }
                Err(_) => {
                    store.error = Some("代理端口需为 1024–65535 之间的整数".into());
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            hint(ui, "范围 1024–65535；运行中修改会自动重启内核生效。");
        });
    });
}

fn interval_text(value: u64) -> String {
    match value {
        1 => "1 秒".into(),
        5 => "5 秒".into(),
        15 => "15 秒".into(),
        60 => "1 分钟".into(),
        300 => "5 分钟".into(),
        other => format!("{other} 秒"),
    }
}
