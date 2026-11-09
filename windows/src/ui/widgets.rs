use egui::{Color32, Margin, RichText, Rounding, Stroke, Ui};

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
pub const ON_ACCENT: Color32 = Color32::WHITE;
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(170, 178, 192);
pub const TEXT_HOVER: Color32 = Color32::from_rgb(196, 203, 216);
pub const TEXT_NAV: Color32 = Color32::from_rgb(160, 168, 184);
pub const TRANSPARENT: Color32 = Color32::TRANSPARENT;

pub const CONTROL_HEIGHT: f32 = 32.0;
pub const CONTROL_GAP: f32 = 8.0;
pub const SWITCH_WIDTH: f32 = 44.0;
pub const SWITCH_TRACK_HEIGHT: f32 = 24.0;

pub const SURFACE: Color32 = Color32::from_rgb(24, 27, 36);
pub const SURFACE_ALT: Color32 = Color32::from_rgb(31, 35, 46);
pub const BORDER: Color32 = Color32::from_rgb(44, 49, 64);
pub const SIDEBAR_BG: Color32 = Color32::from_rgb(17, 19, 26);

// ---- 基础组件 -------------------------------------------------------------

pub fn card_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0_f32, BORDER))
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
        ui.painter().rect_filled(
            bar.rect.shrink2(egui::vec2(0.0, 1.0)),
            Rounding::same(2.0),
            ACCENT,
        );
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

/// 单一操作进度入口；仅展示真实的忙碌状态，不影响页面上的停止按钮。
pub fn operation_progress(ui: &mut Ui, busy: bool, stopping: bool) {
    if !busy && !stopping {
        return;
    }
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(16.0).color(ACCENT));
        hint(
            ui,
            if stopping {
                "正在停止并恢复网络…"
            } else {
                "正在处理，请稍候… 可随时停止并恢复网络"
            },
        );
    });
    ui.add_space(8.0);
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
pub fn primary_button(
    ui: &mut Ui,
    text: &str,
    fill: Color32,
    enabled: bool,
    size: egui::Vec2,
) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).size(15.0).strong().color(ON_ACCENT))
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
            .stroke(Stroke::new(1.0_f32, ACCENT)),
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
        TRANSPARENT
    };
    ui.painter().rect_filled(rect, Rounding::same(8.0), fill);
    if selected {
        let indicator = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 10.0, rect.top() + 9.0),
            egui::vec2(3.0, rect.height() - 18.0),
        );
        ui.painter()
            .rect_filled(indicator, Rounding::same(2.0), ACCENT);
    }
    let text_color = if selected {
        TEXT
    } else if response.hovered() {
        TEXT_HOVER
    } else {
        TEXT_NAV
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

/// 只读实际状态，点击交由调用方派发 Action，不预先翻转状态。
pub fn switch(ui: &mut Ui, label: &str, active: bool, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(SWITCH_WIDTH, CONTROL_HEIGHT),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), active, label)
        });
        if ui.is_rect_visible(rect) {
            let track = egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(SWITCH_WIDTH, SWITCH_TRACK_HEIGHT),
            );
            let fill = if active { ACCENT } else { SURFACE_ALT };
            let border = if response.has_focus() || response.hovered() {
                ACCENT
            } else {
                BORDER
            };
            ui.painter().rect(
                track,
                Rounding::same(12.0),
                fill,
                Stroke::new(1.0_f32, border),
            );
            let thumb_x = if active {
                track.right() - 12.0
            } else {
                track.left() + 12.0
            };
            ui.painter().circle_filled(
                egui::pos2(thumb_x, track.center().y),
                9.0,
                if active { ON_ACCENT } else { TEXT_SECONDARY },
            );
            if response.has_focus() {
                ui.painter()
                    .rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0_f32, ACCENT));
            }
        }
        response
    })
    .inner
}

pub fn proxy_switch(
    ui: &mut Ui,
    title: &str,
    detail: &str,
    active: bool,
    enabled: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = CONTROL_GAP;
        ui.label(RichText::new(title).size(14.0).strong().color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = switch(ui, title, active, enabled)
                .on_hover_text(detail)
                .on_disabled_hover_text(detail);
            hint(ui, if active { "已开启" } else { "已关闭" });
            response
        })
        .inner
    })
    .inner
}

/// 固定三等分，避免文字长度或选中状态改变按钮宽度。
fn mode_options(
    ui: &mut Ui,
    labels: [&str; 3],
    selected: usize,
    enabled: bool,
) -> [egui::Response; 3] {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = CONTROL_GAP;
        ui.add_enabled_ui(enabled, |ui| {
            ui.columns(3, |columns| {
                std::array::from_fn(|index| {
                    let active = selected == index;
                    let width = columns[index].available_width();
                    columns[index].add_sized(
                        egui::vec2(width, CONTROL_HEIGHT),
                        egui::Button::new(
                            RichText::new(labels[index]).size(13.0).color(if active {
                                TEXT
                            } else {
                                TEXT_SECONDARY
                            }),
                        )
                        .truncate()
                        .selected(active)
                        .fill(if active { ACCENT_SOFT } else { SURFACE_ALT })
                        .stroke(Stroke::new(1.0_f32, if active { ACCENT } else { BORDER })),
                    )
                })
            })
        })
        .inner
    })
    .inner
}

pub fn controls_enabled(store: &crate::state::Store) -> bool {
    !store.busy && !store.recovery && !store.stopping && !store.is_testing_latency
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
    let interactive = controls_enabled(store);
    card(ui, Some("代理内核"), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for candidate in KernelKind::ALL {
                let active = current == candidate;
                let fill = if active { ACCENT_SOFT } else { SURFACE_ALT };
                let stroke_color = if active { ACCENT } else { BORDER };
                let text_color = if active { TEXT } else { TEXT_SECONDARY };
                if ui
                    .add_enabled(
                        interactive,
                        egui::Button::new(
                            RichText::new(candidate.title())
                                .size(13.0)
                                .color(text_color),
                        )
                        .fill(fill)
                        .stroke(Stroke::new(1.0_f32, stroke_color)),
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
        facts.push(if caps.tun {
            "TUN 可用"
        } else {
            "不支持 TUN"
        });
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

/// 代理模式仍只派发既有 Action，由业务层应用并保存。
pub fn traffic_mode_selector(
    store: &crate::state::Store,
    ui: &mut Ui,
    pending: &mut Vec<super::Action>,
) {
    use crate::logic::model::TrafficMode;
    let current = store.preferences.traffic_mode;
    let responses = ui
        .horizontal(|ui| {
            ui.add_sized(
                egui::vec2(64.0, CONTROL_HEIGHT),
                egui::Label::new(RichText::new("代理模式").size(13.0).strong().color(TEXT)),
            );
            mode_options(
                ui,
                TrafficMode::ALL.map(|mode| mode.title()),
                TrafficMode::ALL
                    .iter()
                    .position(|mode| *mode == current)
                    .unwrap(),
                controls_enabled(store),
            )
        })
        .inner;
    for (candidate, response) in TrafficMode::ALL.into_iter().zip(responses) {
        if response.on_hover_text(candidate.explanation()).clicked() && current != candidate {
            pending.push(super::Action::TrafficMode(candidate));
        }
    }
}

pub fn traffic_mode_details(
    store: &crate::state::Store,
    ui: &mut Ui,
    pending: &mut Vec<super::Action>,
) {
    card(ui, Some("代理模式与规则维护"), |ui| {
        hint(ui, store.preferences.traffic_mode.explanation());
        let caps = store.preferences.kernel.capabilities();
        hint(
            ui,
            if store.running {
                if caps.mode_switch_restarts {
                    "切换代理模式会重启内核，已有请求可能需要重试。"
                } else {
                    "切换代理模式会关闭旧连接，已有请求可能需要重试。"
                }
            } else {
                "模式将在启动内核时写入配置。"
            },
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.hyperlink_to(
                RichText::new("规则源：MrTager/SmartClash-Rules").size(12.0),
                crate::logic::configuration::RULES_REPOSITORY,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_update = store.running
                    && controls_enabled(store)
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

/// 链路选择只保存偏好，重新选路仍由单独按钮触发。
pub fn routing_selector(
    store: &mut crate::state::Store,
    ui: &mut Ui,
    pending: &mut Vec<super::Action>,
) {
    use crate::logic::model::RoutingMode;
    let mode = store.preferences.mode();
    let responses = ui
        .horizontal(|ui| {
            ui.add_sized(
                egui::vec2(64.0, CONTROL_HEIGHT),
                egui::Label::new(RichText::new("链路模式").size(13.0).strong().color(TEXT)),
            );
            let button_width = 80.0;
            let options_width = (ui.available_width() - button_width - CONTROL_GAP).max(0.0);
            let options = ui
                .allocate_ui_with_layout(
                    egui::vec2(options_width, CONTROL_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        mode_options(
                            ui,
                            RoutingMode::ALL.map(|mode| mode.title()),
                            RoutingMode::ALL
                                .iter()
                                .position(|candidate| *candidate == mode)
                                .unwrap(),
                            controls_enabled(store),
                        )
                    },
                )
                .inner;
            if ui
                .add_enabled(
                    store.preferences.traffic_mode.uses_proxy()
                        && store.running
                        && controls_enabled(store),
                    egui::Button::new(RichText::new("重新选路").size(12.0).color(ACCENT))
                        .fill(ACCENT_DIM)
                        .stroke(Stroke::new(1.0, ACCENT))
                        .min_size(egui::vec2(button_width, CONTROL_HEIGHT)),
                )
                .clicked()
            {
                pending.push(super::Action::ApplyMode);
            }
            options
        })
        .inner;
    for (candidate, response) in RoutingMode::ALL.into_iter().zip(responses) {
        if response.on_hover_text(candidate.explanation()).clicked() {
            store.preferences.routing_mode = Some(candidate);
            store.save_preferences();
        }
    }
    hint(
        ui,
        &routing_status_text(
            store.running,
            store.stopping,
            store.recovery,
            store.preferences.traffic_mode.uses_proxy(),
            store.preferences.mode(),
            store.active_mode,
            &store.routing_status,
            &store.internet_status,
        ),
    );
}

fn routing_status_text(
    running: bool,
    stopping: bool,
    recovery: bool,
    uses_proxy: bool,
    selected: crate::logic::model::RoutingMode,
    active: Option<crate::logic::model::RoutingMode>,
    routing_status: &str,
    internet_status: &str,
) -> String {
    let state = if stopping {
        "正在停止并恢复网络".into()
    } else if recovery {
        "网络设置待恢复，暂不确认生效链路".into()
    } else if !running {
        "内核已停止".into()
    } else if !uses_proxy {
        "当前直连 · 不使用代理链路".into()
    } else if let Some(active) = active {
        if internet_status.contains("出网失败") {
            format!("当前配置：{} · 出网失败，正在检查备用", active.title())
        } else if routing_status == "已打通" {
            format!("当前生效：{}", active.title())
        } else {
            format!("当前配置：{} · 正在重新验证", active.title())
        }
    } else {
        "当前链路尚未验证".into()
    };
    if running && !stopping && !recovery && uses_proxy {
        if active != Some(selected) {
            format!("{state} · 已选{}，待重新选路", selected.title())
        } else {
            state
        }
    } else {
        format!("{state} · 链路偏好已保存")
    }
}

fn routing_status_row(store: &crate::state::Store, ui: &mut Ui, pending: &mut Vec<super::Action>) {
    ui.horizontal_wrapped(|ui| {
        if accent_button(
            ui,
            "重新选路",
            store.preferences.traffic_mode.uses_proxy() && store.running && controls_enabled(store),
        ) {
            pending.push(super::Action::ApplyMode);
        }
        hint(
            ui,
            &routing_status_text(
                store.running,
                store.stopping,
                store.recovery,
                store.preferences.traffic_mode.uses_proxy(),
                store.preferences.mode(),
                store.active_mode,
                &store.routing_status,
                &store.internet_status,
            ),
        );
    });
}

pub fn routing_details(store: &crate::state::Store, ui: &mut Ui) {
    card(ui, Some("链路详情"), |ui| {
        hint(ui, store.preferences.mode().explanation());
        if !store.preferences.traffic_mode.uses_proxy() {
            hint(
                ui,
                "直连模式下链路选择不生效；切换到规则或全局代理后才会使用节点。",
            );
        }
        hint(ui, &store.routing_status);
        if store.running && store.preferences.traffic_mode.uses_proxy() {
            selected_node_details(store, ui);
        }
    });
}

/// 线路模式只读摘要：链式代理页用于确认当前模式并重新选路，切换入口仍在概览页。
pub fn routing_summary(
    store: &mut crate::state::Store,
    ui: &mut Ui,
    pending: &mut Vec<super::Action>,
) {
    card(ui, Some("线路模式"), |ui| {
        routing_status_row(store, ui, pending);
        hint(ui, &format!("已选：{}", store.preferences.mode().title()));
        hint(ui, store.preferences.mode().explanation());
        if !store.preferences.traffic_mode.uses_proxy() {
            hint(
                ui,
                "直连模式下链路选择不生效；切换到规则或全局代理后才会使用节点。",
            );
        }
        hint(ui, "切换链路模式请在概览页操作。");
        if store.running && store.preferences.traffic_mode.uses_proxy() {
            selected_node_details(store, ui);
        }
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
                hint(
                    ui,
                    &format!("下载抽测：{:.2} Mbps", speed.megabits_per_second()),
                );
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
                            .color(WARN)
                            .name("上传")
                            .width(1.8_f32),
                    );
                    plot_ui.line(
                        Line::new(PlotPoints::from(download))
                            .color(ACCENT)
                            .name("下载")
                            .width(1.8_f32),
                    );
                }
            });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("●")
                    .size(8.0)
                    .color(if data.connected { GOOD } else { MUTED }),
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Response};

    fn frame<R>(
        ctx: &Context,
        width: f32,
        events: Vec<Event>,
        mut draw: impl FnMut(&mut Ui) -> R,
    ) -> (R, egui::FullOutput) {
        let mut result = None;
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 600.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| result = Some(draw(ui)));
            },
        );
        (result.unwrap(), output)
    }

    fn spinner_count(output: &egui::FullOutput) -> usize {
        output
            .shapes
            .iter()
            .filter(|shape| {
                matches!(
                    &shape.shape,
                    egui::epaint::Shape::Path(path)
                        if !path.closed && path.points.len() == 20
                            && path.stroke == egui::epaint::PathStroke::new(3.0_f32, ACCENT)
                )
            })
            .count()
    }

    #[test]
    fn operation_progress_has_one_spinner_and_clears_when_idle() {
        let ctx = Context::default();
        for (busy, stopping, expected) in [
            (false, false, 0),
            (true, false, 1),
            (true, true, 1),
            (false, true, 1),
            (false, false, 0),
        ] {
            let (_, output) = frame(&ctx, 660.0, vec![], |ui| {
                operation_progress(ui, busy, stopping);
            });
            assert_eq!(spinner_count(&output), expected);
        }
    }

    #[test]
    fn operation_progress_keeps_stop_clickable_and_uses_stopping_message() {
        let ctx = Context::default();
        let (stop, _) = frame(&ctx, 660.0, vec![], |ui| {
            operation_progress(ui, true, false);
            ui.button("停止并恢复网络")
        });
        let (stop, _) = frame(&ctx, 660.0, click_events(stop.rect.center()), |ui| {
            operation_progress(ui, true, false);
            ui.button("停止并恢复网络")
        });
        assert!(stop.enabled() && stop.clicked());
        let (_, output) = frame(&ctx, 660.0, vec![], |ui| {
            operation_progress(ui, true, true);
        });
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::epaint::Shape::Text(text)
                if text.galley.job.text == "正在停止并恢复网络…"
        )));
    }

    #[test]
    fn operation_progress_is_drawn_once_in_content_not_toolbar_or_chain() {
        let shell = include_str!("mod.rs");
        let (toolbar, content) = shell.split_once("egui::CentralPanel::default()").unwrap();
        assert!(!toolbar.contains("ui.spinner()"));
        assert!(!toolbar.contains("operation_progress("));
        assert_eq!(content.matches("widgets::operation_progress(").count(), 1);
        assert!(!include_str!("chain.rs").contains("ui.spinner()"));
        assert!(!include_str!("chain.rs").contains("operation_progress("));
    }

    fn click_events(pos: Pos2) -> Vec<Event> {
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ]
    }

    fn key_event(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn switch_frame(
        ctx: &Context,
        active: bool,
        enabled: bool,
        events: Vec<Event>,
    ) -> (Response, egui::FullOutput) {
        frame(ctx, 400.0, events, |ui| {
            switch(ui, "系统代理", active, enabled)
        })
    }

    #[test]
    fn switch_click_reports_actual_state_without_optimistic_change() {
        for active in [false, true] {
            let ctx = Context::default();
            let (initial, _) = switch_frame(&ctx, active, true, vec![]);
            assert_eq!(
                initial.rect.size(),
                egui::vec2(SWITCH_WIDTH, CONTROL_HEIGHT)
            );
            let (clicked, output) =
                switch_frame(&ctx, active, true, click_events(initial.rect.center()));
            assert!(clicked.clicked());
            assert!(!clicked.changed());
            let info = output
                .platform_output
                .events
                .iter()
                .find_map(|event| match event {
                    egui::output::OutputEvent::Clicked(info) => Some(info),
                    _ => None,
                })
                .expect("点击应包含无障碍信息");
            assert_eq!(info.typ, egui::WidgetType::Checkbox);
            assert_eq!(info.label.as_deref(), Some("系统代理"));
            assert_eq!(info.selected, Some(active));
            let (unchanged, _) = switch_frame(&ctx, active, true, vec![]);
            assert!(!unchanged.clicked());
        }
    }

    #[test]
    fn switch_supports_keyboard_focus_and_activation() {
        for key in [Key::Space, Key::Enter] {
            let ctx = Context::default();
            let _ = switch_frame(&ctx, false, true, vec![]);
            let (focused, _) = switch_frame(&ctx, false, true, vec![key_event(Key::Tab)]);
            assert!(focused.has_focus(), "Tab 应将焦点移到开关");
            let (clicked, _) = switch_frame(&ctx, false, true, vec![key_event(key)]);
            assert!(clicked.clicked(), "空格和回车应触发点击");
        }
    }

    #[test]
    fn disabled_switch_rejects_pointer_keyboard_and_parent_disabled() {
        for active in [false, true] {
            let ctx = Context::default();
            let (initial, _) = switch_frame(&ctx, active, false, vec![]);
            let (disabled, _) =
                switch_frame(&ctx, active, false, click_events(initial.rect.center()));
            assert!(!disabled.enabled());
            assert!(!disabled.clicked());

            let (enabled, _) = switch_frame(&ctx, active, true, vec![]);
            enabled.request_focus();
            let (disabled, _) = switch_frame(&ctx, active, false, vec![key_event(Key::Space)]);
            assert!(!disabled.clicked());
            assert!(!disabled.has_focus());

            let (parent_disabled, _) = frame(&ctx, 400.0, vec![], |ui| {
                ui.add_enabled_ui(false, |ui| switch(ui, "TUN", active, true))
                    .inner
            });
            assert!(!parent_disabled.enabled());
        }
    }

    #[test]
    fn proxy_switch_columns_stay_equal_and_inside_available_width() {
        for width in [360.0, 660.0, 820.0] {
            let ctx = Context::default();
            let ((bounds, columns, switches), _) = frame(&ctx, width, vec![], |ui| {
                let bounds = ui.available_rect_before_wrap();
                ui.spacing_mut().item_spacing.x = CONTROL_GAP;
                let mut columns_bounds = [Rect::NOTHING; 2];
                let switches = ui.columns(2, |columns| {
                    std::array::from_fn::<_, 2, _>(|index| {
                        columns_bounds[index] = columns[index].max_rect();
                        proxy_switch(
                            &mut columns[index],
                            ["系统代理", "TUN 虚拟网卡"][index],
                            "说明",
                            index == 0,
                            true,
                        )
                        .rect
                    })
                });
                (bounds, columns_bounds, switches)
            });
            assert!((columns[0].width() - columns[1].width()).abs() < 0.1);
            for (column, switch) in columns.into_iter().zip(switches) {
                assert!(switch.left() >= column.left());
                assert!(switch.right() <= column.right() + 0.1);
                assert!(switch.right() <= bounds.right() + 0.1);
                assert_eq!(switch.size(), egui::vec2(SWITCH_WIDTH, CONTROL_HEIGHT));
            }
        }
    }

    #[test]
    fn mode_buttons_fill_three_equal_columns_at_multiple_widths() {
        use crate::logic::model::{RoutingMode, TrafficMode};
        for width in [360.0, 660.0, 820.0] {
            for labels in [
                TrafficMode::ALL.map(|mode| mode.title()),
                RoutingMode::ALL.map(|mode| mode.title()),
            ] {
                for selected in 0..3 {
                    let ctx = Context::default();
                    let ((bounds, responses), _) = frame(&ctx, width, vec![], |ui| {
                        let bounds = ui.available_rect_before_wrap();
                        (bounds, mode_options(ui, labels, selected, true))
                    });
                    let expected = (bounds.width() - CONTROL_GAP * 2.0) / 3.0;
                    for response in &responses {
                        assert!((response.rect.width() - expected).abs() < 0.1);
                        assert!((response.rect.height() - CONTROL_HEIGHT).abs() < 0.1);
                        assert!(response.rect.left() >= bounds.left() - 0.1);
                        assert!(response.rect.right() <= bounds.right() + 0.1);
                    }
                    for pair in responses.windows(2) {
                        assert!(
                            (pair[1].rect.left() - pair[0].rect.right() - CONTROL_GAP).abs() < 0.1
                        );
                        assert_eq!(pair[0].rect.top(), pair[1].rect.top());
                    }
                    let ((_, clicked), _) = frame(
                        &ctx,
                        width,
                        click_events(responses[1].rect.center()),
                        |ui| {
                            (
                                ui.available_rect_before_wrap(),
                                mode_options(ui, labels, selected, true),
                            )
                        },
                    );
                    assert!(clicked[1].clicked());
                    let (disabled, _) = frame(
                        &ctx,
                        width,
                        click_events(responses[1].rect.center()),
                        |ui| mode_options(ui, labels, selected, false),
                    );
                    assert!(disabled
                        .iter()
                        .all(|response| !response.enabled() && !response.clicked()));
                }
            }
        }
    }

    #[test]
    fn routing_status_distinguishes_pending_direct_stopped_and_recovery() {
        use crate::logic::model::RoutingMode::{Ai, LowLatency};
        let pending = routing_status_text(
            true, false, false, true, Ai, Some(LowLatency), "已打通", "HTTPS 出网正常",
        );
        assert!(pending.contains("当前生效：低延迟链路"));
        assert!(pending.contains("已选AI 链路，待重新选路"));
        let active = routing_status_text(
            true, false, false, true, Ai, Some(Ai), "已打通", "HTTPS 出网正常",
        );
        assert!(!active.contains("待重新选路"));
        let verifying = routing_status_text(
            true, false, false, true, Ai, Some(Ai), "正在验证AI 链路", "HTTPS 出网正常",
        );
        assert!(verifying.contains("当前配置：AI 链路 · 正在重新验证"));
        let failed = routing_status_text(
            true, false, false, true, Ai, Some(Ai), "已打通", "当前线路出网失败，正在检查备用",
        );
        assert!(failed.contains("当前配置：AI 链路 · 出网失败"));
        for (running, stopping, recovery, uses_proxy, expected) in [
            (true, false, false, false, "当前直连"),
            (false, false, false, true, "内核已停止"),
            (true, true, false, true, "正在停止并恢复网络"),
            (true, false, true, true, "网络设置待恢复"),
        ] {
            let text = routing_status_text(
                running, stopping, recovery, uses_proxy, Ai, None, "尚未打通", "未连接",
            );
            assert!(text.contains(expected));
            assert!(!text.contains("当前生效"));
            assert!(!text.contains("待重新选路"));
            assert!(text.contains("链路偏好已保存"));
        }
    }

    #[test]
    fn compact_mode_rows_fit_minimum_window_content_with_cjk_font() {
        for width in [620.0, 660.0, 820.0] {
            let ctx = Context::default();
            crate::install_fonts(&ctx);
            for with_reroute in [false, true] {
                let ((bounds, row), _) = frame(&ctx, width, vec![], |ui| {
                    let bounds = ui.available_rect_before_wrap();
                    let row = ui
                        .horizontal(|ui| {
                            ui.add_sized(
                                egui::vec2(64.0, CONTROL_HEIGHT),
                                egui::Label::new("链路模式"),
                            );
                            let options_width = ui.available_width()
                                - if with_reroute {
                                    80.0 + CONTROL_GAP
                                } else {
                                    0.0
                                };
                            ui.allocate_ui_with_layout(
                                egui::vec2(options_width, CONTROL_HEIGHT),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    mode_options(ui, ["低延迟链路", "高速链路", "AI 链路"], 0, true)
                                },
                            );
                            if with_reroute {
                                ui.add_sized(
                                    egui::vec2(80.0, CONTROL_HEIGHT),
                                    egui::Button::new("重新选路"),
                                );
                            }
                        })
                        .response
                        .rect;
                    (bounds, row)
                });
                assert!(
                    row.right() <= bounds.right() + 0.1,
                    "row overflow: {row:?} / {bounds:?}"
                );
                assert!(row.height() <= CONTROL_HEIGHT + 0.1);
            }
        }
    }
}
