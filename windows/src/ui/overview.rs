use super::widgets::{self, hint};
use super::{Action, ViewState};
use crate::state::Store;
use egui::{Margin, RichText, Rounding, Stroke, Ui};

pub fn draw(
    view: &mut ViewState,
    store: &mut Store,
    ui: &mut Ui,
    pending: &mut Vec<Action>,
) {
    // ---- 英雄区 ----
    egui::Frame::default()
        .fill(widgets::HERO_BG)
        .stroke(Stroke::new(1.0, widgets::HERO_BORDER))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(18.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("让连接持续在线").size(24.0).strong().color(widgets::TEXT));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (fg, bg) = widgets::status_color(&store.internet_status);
                        widgets::status_pill(ui, &store.internet_status, fg, bg);
                        let (fg, bg) = widgets::status_color(&store.openai_status);
                        widgets::status_pill(ui, &store.openai_status, fg, bg);
                    });
                    ui.add_space(12.0);
                    let running = store.running || store.busy;
                    let (label, fill) = if running {
                        ("停止并恢复网络", widgets::BAD)
                    } else {
                        ("一键打通", widgets::ACCENT)
                    };
                    if widgets::primary_button(
                        ui,
                        label,
                        fill,
                        !store.stopping,
                        egui::vec2(184.0, 40.0),
                    ) {
                        pending.push(if running {
                            Action::Stop
                        } else {
                            Action::Connect
                        });
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    if store.busy {
                        ui.vertical(|ui| {
                            ui.spinner();
                            hint(ui, "正在执行操作，可随时停止");
                        });
                    }
                });
            });
        });
    ui.add_space(10.0);

    if store.recovery {
        widgets::card_frame()
            .fill(widgets::WARN_DIM)
            .stroke(Stroke::new(1.0, widgets::WARN))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("● 检测到未恢复的网络设置")
                            .size(13.0)
                            .strong()
                            .color(widgets::WARN),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !store.stopping,
                                egui::Button::new(
                                    RichText::new("恢复网络").size(13.0).color(widgets::WARN),
                                )
                                .stroke(Stroke::new(1.0, widgets::WARN)),
                            )
                            .clicked()
                        {
                            pending.push(Action::Stop);
                        }
                    });
                });
            });
        ui.add_space(10.0);
    }

    // ---- 指标 ----
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        widgets::metric(ui, "聚合节点", &store.nodes.len().to_string(), widgets::ACCENT);
        widgets::metric(
            ui,
            "内核状态",
            if store.running { "运行中" } else { "已停止" },
            if store.running { widgets::GOOD } else { widgets::MUTED },
        );
        widgets::metric(
            ui,
            "当前出口",
            if !store.preferences.traffic_mode.uses_proxy() {
                "直连"
            } else if store.selected.is_empty() {
                "尚未连接"
            } else {
                store.selected.as_str()
            },
            if !store.preferences.traffic_mode.uses_proxy() {
                widgets::MUTED
            } else {
                widgets::GOOD
            },
        );
    });
    ui.add_space(6.0);

    widgets::kernel_controls(store, ui, pending);
    ui.add_space(2.0);

    widgets::traffic_mode_controls(store, ui, pending);
    ui.add_space(2.0);

    super::globe::draw(store, ui, view);

    widgets::routing_controls(store, ui, pending);
    ui.add_space(4.0);

    widgets::card(ui, Some("代理控制"), |ui| {
        hint(ui, "系统代理与 TUN 可以同时开启，分别开关互不影响。");
        ui.add_space(8.0);
        let can_toggle = !store.busy && !store.recovery && !store.stopping;
        if widgets::toggle_row(
            ui,
            "系统代理",
            "开启 Windows 系统 HTTP / HTTPS 代理（写入当前用户注册表）。",
            store.system_proxy,
            "一键开启",
            "关闭",
            can_toggle,
        ) {
            pending.push(Action::ToggleSystem);
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
        let tun_supported = store.preferences.kernel.capabilities().tun;
        if widgets::toggle_row(
            ui,
            "TUN 虚拟网卡",
            if tun_supported {
                "接管更多应用流量；开启和关闭都需要管理员授权（每次弹 UAC）。"
            } else {
                "当前内核不支持 TUN；请切换到 Mihomo 或 sing-box。"
            },
            store.tun,
            "一键开启 TUN",
            "关闭 TUN",
            can_toggle && tun_supported,
        ) {
            pending.push(Action::ToggleTun);
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !store.stopping && (store.running || !store.recovery),
                    egui::Button::new(if store.running {
                        "停止内核并恢复网络"
                    } else {
                        "仅启动内核"
                    }),
                )
                .clicked()
            {
                pending.push(if store.running {
                    Action::Stop
                } else {
                    Action::StartOnly
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                hint(
                    ui,
                    &format!("HTTP / SOCKS · 127.0.0.1:{}", crate::logic::model::proxy_port()),
                );
            });
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
        port_controls(view, store, ui, pending);
    });

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
                        store.running && !store.busy,
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
fn port_controls(
    view: &mut ViewState,
    store: &mut Store,
    ui: &mut Ui,
    pending: &mut Vec<Action>,
) {
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
        let enter = input.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter));
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
