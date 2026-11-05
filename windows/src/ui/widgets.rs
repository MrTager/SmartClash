use egui::{Color32, Margin, Rounding, RichText, Stroke, Ui};

// ---- 调色板 ---------------------------------------------------------------
pub const ACCENT: Color32 = Color32::from_rgb(108, 140, 255);
// 半透明替换色（卡片上的浅色底）：直接用偏色的深色调，避免预乘 alpha 换算
pub const ACCENT_DIM: Color32 = Color32::from_rgb(34, 39, 58);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(46, 54, 82);
pub const GOOD: Color32 = Color32::from_rgb(72, 214, 149);
pub const GOOD_DIM: Color32 = Color32::from_rgb(26, 46, 40);
pub const WARN: Color32 = Color32::from_rgb(246, 184, 82);
pub const WARN_DIM: Color32 = Color32::from_rgb(48, 42, 30);
pub const BAD: Color32 = Color32::from_rgb(245, 112, 112);
pub const BAD_DIM: Color32 = Color32::from_rgb(48, 32, 32);
pub const MUTED: Color32 = Color32::from_rgb(143, 151, 170);
pub const MUTED_DIM: Color32 = Color32::from_rgb(32, 35, 42);
pub const TEXT: Color32 = Color32::from_rgb(228, 232, 240);

pub const SURFACE: Color32 = Color32::from_rgb(24, 27, 36);
pub const SURFACE_ALT: Color32 = Color32::from_rgb(31, 35, 46);
pub const BORDER: Color32 = Color32::from_rgb(44, 49, 64);
pub const SIDEBAR_BG: Color32 = Color32::from_rgb(17, 19, 26);
pub const HERO_BG: Color32 = Color32::from_rgb(27, 31, 45);
pub const HERO_BORDER: Color32 = Color32::from_rgb(58, 66, 94);

// ---- 基础组件 -------------------------------------------------------------

pub fn card_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::same(14.0))
}

pub fn card(ui: &mut Ui, title: Option<&str>, add: impl FnOnce(&mut Ui)) {
    card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let Some(title) = title {
            card_title(ui, title);
        }
        add(ui);
    });
    ui.add_space(4.0);
}

fn card_title(ui: &mut Ui, title: &str) {
    ui.horizontal(|ui| {
        let bar = ui.allocate_response(egui::vec2(4.0, 14.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(bar.rect.shrink2(egui::vec2(0.0, 1.0)), Rounding::same(2.0), ACCENT);
        ui.label(RichText::new(title).size(15.0).strong().color(TEXT));
    });
    ui.add_space(10.0);
}

pub fn page_title(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(24.0).strong().color(TEXT));
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).size(12.5).color(MUTED));
    }
    ui.add_space(12.0);
}

pub fn metric(ui: &mut Ui, title: &str, value: &str, color: Color32) {
    card_frame().show(ui, |ui| {
        ui.set_min_width(180.0);
        ui.label(RichText::new(title).size(11.0).color(MUTED));
        ui.add_space(3.0);
        ui.label(RichText::new(value).size(18.0).strong().color(color));
    });
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED));
}

/// 圆角状态胶囊：小圆点 + 文本
pub fn status_pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::default()
        .fill(bg)
        .rounding(Rounding::same(999.0))
        .inner_margin(egui::Margin::symmetric(10.0, 4.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("●").size(8.0).color(fg));
                ui.label(RichText::new(text).size(12.0).color(fg));
            });
        });
}

/// 根据状态文本推断配色
pub fn status_color(text: &str) -> (Color32, Color32) {
    if text.contains("已验证")
        || text.contains("已确认")
        || text.contains("可达")
        || text.contains("正常")
        || text.contains("已打通")
    {
        (GOOD, GOOD_DIM)
    } else if text.contains("失败") || text.contains("不可用") || text.contains("未通过") {
        (BAD, BAD_DIM)
    } else if text.contains("尚未") || text.contains("未连接") || text.contains("等待") {
        (MUTED, MUTED_DIM)
    } else {
        (ACCENT, ACCENT_DIM)
    }
}

/// 主要操作按钮（填充强调色）
pub fn primary_button(ui: &mut Ui, text: &str, fill: Color32, enabled: bool, size: egui::Vec2) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).size(15.0).strong().color(Color32::WHITE))
            .fill(fill)
            .stroke(Stroke::NONE)
            .min_size(size),
    )
    .clicked()
}

/// 次要强调按钮（描边强调色）
pub fn accent_button(ui: &mut Ui, text: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).size(13.0).color(ACCENT))
            .fill(ACCENT_DIM)
            .stroke(Stroke::new(1.0, ACCENT)),
    )
    .clicked()
}

/// 侧栏导航项：选中时强调色胶囊 + 左侧指示条
pub fn nav_item(ui: &mut Ui, title: &str, selected: bool) -> bool {
    let width = ui.available_width();
    let response = ui.allocate_response(egui::vec2(width, 34.0), egui::Sense::click());
    let rect = response.rect;
    let fill = if selected {
        ACCENT_DIM
    } else if response.hovered() {
        SURFACE_ALT
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, Rounding::same(8.0), fill);
    if selected {
        let indicator = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 10.0, rect.top() + 9.0),
            egui::vec2(3.0, rect.height() - 18.0),
        );
        ui.painter().rect_filled(indicator, Rounding::same(2.0), ACCENT);
    }
    let text_color = if selected {
        TEXT
    } else if response.hovered() {
        Color32::from_rgb(196, 203, 216)
    } else {
        Color32::from_rgb(160, 168, 184)
    };
    ui.painter().text(
        egui::pos2(rect.left() + 22.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(14.0),
        text_color,
    );
    response.clicked()
}

/// 开关行：标题 + 说明 + 状态胶囊 + 操作按钮
pub fn toggle_row(
    ui: &mut Ui,
    title: &str,
    detail: &str,
    active: bool,
    on_label: &str,
    off_label: &str,
    enabled: bool,
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(15.0).strong().color(TEXT));
            if !detail.is_empty() {
                hint(ui, detail);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if active {
                clicked = ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(RichText::new(off_label).size(13.0).color(BAD)).fill(
                            Color32::TRANSPARENT,
                        ),
                    )
                    .clicked();
            } else {
                clicked = ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(
                            RichText::new(on_label).size(13.0).strong().color(Color32::WHITE),
                        )
                        .fill(ACCENT)
                        .stroke(Stroke::NONE),
                    )
                    .clicked();
            }
            ui.add_space(6.0);
            if active {
                status_pill(ui, "已开启", GOOD, GOOD_DIM);
            } else {
                status_pill(
                    ui,
                    "已关闭",
                    MUTED,
                    MUTED_DIM,
                );
            }
        });
    });
    clicked
}

// ---- 复合组件 -------------------------------------------------------------

/// 代理内核选择：Mihomo / sing-box / Xray，并说明当前能力差异。
pub fn kernel_controls(
    store: &mut crate::state::Store,
    ui: &mut Ui,
    pending: &mut Vec<super::Action>,
) {
    use crate::logic::kernel::{KernelKind, RulesSupport};
    let current = store.preferences.kernel;
    let interactive = !store.busy && !store.stopping && !store.recovery;
    card(ui, Some("代理内核"), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for candidate in KernelKind::ALL {
                let active = current == candidate;
                let fill = if active { ACCENT_SOFT } else { SURFACE_ALT };
                let stroke_color = if active { ACCENT } else { BORDER };
                let text_color = if active {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(170, 178, 192)
                };
                if ui
                    .add_enabled(
                        interactive,
                        egui::Button::new(
                            RichText::new(candidate.title()).size(13.0).color(text_color),
                        )
                        .fill(fill)
                        .stroke(Stroke::new(1.0, stroke_color)),
                    )
                    .clicked()
                    && !active
                {
                    pending.push(super::Action::Kernel(candidate));
                }
            }
        });
        ui.add_space(6.0);
        hint(ui, current.description());
        ui.add_space(4.0);
        let caps = current.capabilities();
        let mut facts = Vec::new();
        facts.push(if caps.tun { "TUN 可用" } else { "不支持 TUN" });
        facts.push(match caps.rules {
            RulesSupport::Provider => "规则可在线更新",
            RulesSupport::Restart => "规则由内核刷新",
            RulesSupport::Builtin => "内置 geo 规则",
        });
        facts.push(if caps.connections {
            "实时连接可用"
        } else {
            "无实时连接列表"
        });
        facts.push(if caps.live_switch {
            "节点热切换"
        } else {
            "切换节点重启内核"
        });
        hint(ui, &facts.join(" · "));
        if store.running {
            ui.add_space(2.0);
            hint(ui, "切换内核会重启当前内核，已有连接会中断。");
        }
    });
}

/// 流量模式：规则代理 / 全局代理 / 直连，对应 Mihomo 的 mode 字段。
pub fn traffic_mode_controls(
    store: &mut crate::state::Store,
    ui: &mut Ui,
    pending: &mut Vec<super::Action>,
) {
    use crate::logic::model::TrafficMode;
    let current = store.preferences.traffic_mode;
    let interactive = !store.busy && !store.stopping && !store.recovery;
    card(ui, Some("流量模式"), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for candidate in TrafficMode::ALL {
                let active = current == candidate;
                let fill = if active { ACCENT_SOFT } else { SURFACE_ALT };
                let stroke_color = if active { ACCENT } else { BORDER };
                let text_color = if active {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(170, 178, 192)
                };
                if ui
                    .add_enabled(
                        interactive,
                        egui::Button::new(
                            RichText::new(candidate.title()).size(13.0).color(text_color),
                        )
                        .fill(fill)
                        .stroke(Stroke::new(1.0, stroke_color)),
                    )
                    .clicked()
                    && !active
                {
                    pending.push(super::Action::TrafficMode(candidate));
                }
            }
        });
        ui.add_space(6.0);
        hint(ui, current.explanation());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let (fg, bg) = if store.running {
                (ACCENT, ACCENT_DIM)
            } else {
                (MUTED, MUTED_DIM)
            };
            status_pill(
                ui,
                &if store.running {
                    format!("当前生效：{}", current.title())
                } else {
                    "已保存选择，下次连接时生效".into()
                },
                fg,
                bg,
            );
        });
        ui.add_space(4.0);
        let caps = store.preferences.kernel.capabilities();
        hint(
            ui,
            if store.running {
                if caps.mode_switch_restarts {
                    "切换模式会重启内核，已有请求可能需要重试。"
                } else {
                    "切换模式会关闭旧连接，已有请求可能需要重试。"
                }
            } else {
                "尚未连接，模式将在启动内核时写入配置。"
            },
        );
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.hyperlink_to(
                RichText::new("规则源：MrTager/SmartClash-Rules").size(12.0),
                crate::logic::configuration::RULES_REPOSITORY,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_update = store.running
                    && !store.busy
                    && !matches!(caps.rules, crate::logic::kernel::RulesSupport::Builtin);
                if ui
                    .add_enabled(can_update, egui::Button::new("立即更新规则"))
                    .on_disabled_hover_text(match caps.rules {
                        crate::logic::kernel::RulesSupport::Provider => "内核运行时才能更新规则",
                        crate::logic::kernel::RulesSupport::Restart => {
                            "内核运行时才能更新规则（会重启内核）"
                        }
                        crate::logic::kernel::RulesSupport::Builtin => {
                            "Xray 使用内置 geo 数据，无法在线更新"
                        }
                    })
                    .clicked()
                {
                    pending.push(super::Action::UpdateRules);
                }
            });
        });
        ui.add_space(2.0);
        hint(ui, &store.rule_status);
        hint(
            ui,
            match caps.rules {
                crate::logic::kernel::RulesSupport::Provider => {
                    "国内域名与国内 IP 直连规则，经 SMART 组下载，内核运行期间每小时自动刷新；失败保留旧规则。"
                }
                crate::logic::kernel::RulesSupport::Restart => {
                    "国内规则集由 sing-box 按小时自动下载刷新；手动更新会重启内核触发检查。"
                }
                crate::logic::kernel::RulesSupport::Builtin => {
                    "Xray 使用内置 geosite:cn / geoip:cn 分流，规则无法在线更新。"
                }
            },
        );
    });
}

pub fn routing_controls(
    store: &mut crate::state::Store,
    ui: &mut Ui,
    pending: &mut Vec<super::Action>,
) {
    let mode = store.preferences.mode();
    let uses_proxy = store.preferences.traffic_mode.uses_proxy();
    card(ui, Some("线路模式"), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for candidate in crate::logic::model::RoutingMode::ALL {
                let active = mode == candidate;
                let fill = if active { ACCENT_SOFT } else { SURFACE_ALT };
                let stroke_color = if active { ACCENT } else { BORDER };
                let text_color = if active {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(170, 178, 192)
                };
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(candidate.title()).size(13.0).color(text_color),
                        )
                        .fill(fill)
                        .stroke(Stroke::new(1.0, stroke_color)),
                    )
                    .clicked()
                {
                    store.preferences.routing_mode = Some(candidate);
                    store.save_preferences();
                }
            }
        });
        ui.add_space(6.0);
        hint(ui, mode.explanation());
        if !uses_proxy {
            ui.add_space(2.0);
            hint(ui, "直连模式下线路选择不生效；切换到规则或全局代理后才会使用节点。");
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let (fg, bg) = if store.active_mode.is_some() {
                (GOOD, GOOD_DIM)
            } else {
                (MUTED, MUTED_DIM)
            };
            status_pill(
                ui,
                &format!(
                    "当前生效：{}",
                    store
                        .active_mode
                        .map(|m| m.title().to_string())
                        .unwrap_or_else(|| "尚未连接".into())
                ),
                fg,
                bg,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        uses_proxy
                            && store.running
                            && !store.busy
                            && !store.is_testing_latency
                            && !store.stopping,
                        egui::Button::new(RichText::new("按此模式重新选路").size(13.0)),
                    )
                    .clicked()
                {
                    pending.push(super::Action::ApplyMode);
                }
            });
        });
        selected_node_details(store, ui);
    });
}

/// 线路模式只读摘要：链式代理页用于确认当前模式并重新选路，切换入口仍在概览页。
pub fn routing_summary(
    store: &mut crate::state::Store,
    ui: &mut Ui,
    pending: &mut Vec<super::Action>,
) {
    let mode = store.preferences.mode();
    let uses_proxy = store.preferences.traffic_mode.uses_proxy();
    card(ui, Some("线路模式"), |ui| {
        ui.horizontal(|ui| {
            let (fg, bg) = if store.active_mode.is_some() {
                (GOOD, GOOD_DIM)
            } else {
                (MUTED, MUTED_DIM)
            };
            status_pill(
                ui,
                &format!(
                    "当前生效：{}",
                    store
                        .active_mode
                        .map(|m| m.title().to_string())
                        .unwrap_or_else(|| "尚未连接".into())
                ),
                fg,
                bg,
            );
            status_pill(
                ui,
                &format!("已选：{}", mode.title()),
                ACCENT,
                ACCENT_DIM,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        uses_proxy
                            && store.running
                            && !store.busy
                            && !store.is_testing_latency
                            && !store.stopping,
                        egui::Button::new(RichText::new("按此模式重新选路").size(13.0)),
                    )
                    .clicked()
                {
                    pending.push(super::Action::ApplyMode);
                }
            });
        });
        ui.add_space(6.0);
        hint(ui, mode.explanation());
        if !uses_proxy {
            ui.add_space(2.0);
            hint(ui, "直连模式下线路选择不生效；切换到规则或全局代理后才会使用节点。");
        }
        ui.add_space(2.0);
        hint(ui, "切换线路模式请在概览页操作。");
        selected_node_details(store, ui);
    });
}

fn selected_node_details(store: &crate::state::Store, ui: &mut Ui) {
    if let Some(node) = store
        .node_list_nodes()
        .into_iter()
        .find(|n| n.name == store.selected)
    {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if let Some(latency) = store.latencies.get(&node.id).and_then(|l| l.milliseconds) {
                hint(ui, &format!("HTTPS：{latency} ms"));
            }
            if let Some(speed) = store
                .measurements
                .get(&node.id)
                .and_then(|m| m.throughput.as_ref())
            {
                hint(ui, &format!("下载抽测：{:.2} Mbps", speed.megabits_per_second()));
            } else {
                hint(ui, "下载吞吐：未抽测");
            }
            match store
                .measurements
                .get(&node.id)
                .and_then(|m| m.openai_reachable)
            {
                Some(true) => hint(ui, "OpenAI：已验证"),
                Some(false) => hint(ui, "OpenAI：本次未确认"),
                None => {}
            }
        });
    }
}

pub fn traffic_panel(store: &crate::state::Store, ui: &mut Ui) {
    let data = store.traffic.lock().unwrap();
    let format = crate::logic::traffic::format_bytes;
    card(ui, Some("上传与下载流量"), |ui| {
        ui.horizontal(|ui| {
            traffic_totals(
                ui,
                "实时速度",
                &format!("{}/s", format(data.upload_rate)),
                &format!("{}/s", format(data.download_rate)),
            );
            ui.separator();
            traffic_totals(
                ui,
                "本次内核运行累计",
                &crate::logic::traffic::format_bytes(data.session_upload),
                &crate::logic::traffic::format_bytes(data.session_download),
            );
            ui.separator();
            traffic_totals(
                ui,
                "历史累计（本应用记录）",
                &crate::logic::traffic::format_bytes(data.lifetime_upload),
                &crate::logic::traffic::format_bytes(data.lifetime_download),
            );
        });
        ui.add_space(8.0);

        use egui_plot::{Legend, Line, Plot, PlotPoints};
        let upload: Vec<[f64; 2]> = data
            .points
            .iter()
            .enumerate()
            .map(|(index, point)| [index as f64, point.upload as f64])
            .collect();
        let download: Vec<[f64; 2]> = data
            .points
            .iter()
            .enumerate()
            .map(|(index, point)| [index as f64, point.download as f64])
            .collect();
        Plot::new("traffic")
            .height(130.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .include_y(0.0)
            .legend(Legend::default())
            .show(ui, |plot_ui| {
                if !upload.is_empty() {
                    plot_ui.line(
                        Line::new(PlotPoints::from(upload))
                            .color(Color32::from_rgb(246, 184, 82))
                            .name("上传")
                            .width(1.8),
                    );
                    plot_ui.line(
                        Line::new(PlotPoints::from(download))
                            .color(ACCENT)
                            .name("下载")
                            .width(1.8),
                    );
                }
            });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").size(8.0).color(if data.connected { GOOD } else { MUTED }));
            hint(
                ui,
                if data.connected {
                    "内核实时数据 · 最近 60 秒"
                } else {
                    "内核未运行或统计暂不可用"
                },
            );
        });
        hint(
            ui,
            "以本应用内核上报为准，测速和后台探测流量可能计入；不是全电脑流量。历史数据定期保存在本机。",
        );
    });
}

fn traffic_totals(ui: &mut Ui, title: &str, upload: &str, download: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(title).size(11.0).color(MUTED));
        ui.add_space(2.0);
        ui.label(
            RichText::new(format!("↑ {upload}"))
                .size(13.0)
                .strong()
                .color(WARN),
        );
        ui.label(
            RichText::new(format!("↓ {download}"))
                .size(13.0)
                .strong()
                .color(ACCENT),
        );
    });
    ui.add_space(16.0);
}
