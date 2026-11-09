use super::widgets::{self, hint, MUTED};
use super::{Action, ViewState};
use crate::state::Store;
use egui::{Margin, RichText, Rounding, Stroke, Ui};

pub fn draw(
    _view: &mut ViewState,
    store: &mut Store,
    ui: &mut Ui,
    _ctx: &egui::Context,
    pending: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("链路运行情况").size(24.0).strong().color(widgets::TEXT));
            ui.add_space(4.0);
            let failed = store.running && store.internet_status.contains("出网失败");
            let label = if failed {
                "当前链路出网失败 · 正在检查备用"
            } else {
                &store.routing_status
            };
            let (fg, bg) = if !store.running {
                (MUTED, widgets::MUTED_DIM)
            } else if failed {
                (widgets::BAD, widgets::BAD_DIM)
            } else if store.routing_status == "已打通" {
                (widgets::GOOD, widgets::GOOD_DIM)
            } else {
                (widgets::WARN, widgets::WARN_DIM)
            };
            widgets::status_pill(ui, label, fg, bg);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if store.running || store.busy || store.recovery {
                let label = if store.stopping {
                    "正在停止…"
                } else {
                    "停止并恢复网络"
                };
                if widgets::primary_button(
                    ui,
                    label,
                    widgets::BAD,
                    !store.stopping,
                    egui::vec2(168.0, 36.0),
                ) {
                    pending.push(Action::Stop);
                }
            } else if widgets::primary_button(
                ui,
                "连接所选模式",
                widgets::ACCENT,
                !store.is_testing_latency,
                egui::vec2(168.0, 36.0),
            ) {
                pending.push(Action::Connect);
            }
        });
    });
    ui.add_space(8.0);
    widgets::routing_summary(store, ui, pending);

    widgets::card(ui, Some("当前实际生效路径"), |ui| {
        if store.active_trace.hops.is_empty() {
            hint(ui, "尚未连接；连接成功后显示真实节点顺序");
        } else {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    hop(
                        ui,
                        "本机",
                        if store.tun { "TUN" } else { "HTTP / SOCKS" },
                        false,
                    );
                    for (index, node) in store.active_trace.hops.iter().enumerate() {
                        arrow(ui);
                        hop(
                            ui,
                            &node.name.split(" · ").next().unwrap_or(&node.name),
                            &format!("第 {} 跳 · {}", index + 1, node.protocol_name),
                            true,
                        );
                    }
                    arrow(ui);
                    hop(
                        ui,
                        if store.active_mode == Some(crate::logic::model::RoutingMode::Ai) {
                            "OpenAI API"
                        } else {
                            "互联网"
                        },
                        "出网目标",
                        false,
                    );
                });
            });
            ui.add_space(6.0);
            widgets::status_pill(
                ui,
                &format!(
                    "{} 跳 · {} 条近期验证通过的备用线路",
                    store.active_trace.hops.len(),
                    store.standby_ready
                ),
                widgets::TEXT,
                widgets::MUTED_DIM,
            );
            if !store.active_trace.complete {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("路径引用不完整，不能确认全部中间节点")
                        .color(widgets::WARN)
                        .size(12.0),
                );
            }
        }
        ui.add_space(6.0);
        let (fg, bg) = widgets::status_color(&store.internet_status);
        widgets::status_pill(ui, &store.internet_status, fg, bg);
        ui.add_space(4.0);
        let (fg, bg) = widgets::status_color(&store.openai_status);
        widgets::status_pill(ui, &store.openai_status, fg, bg);
    });

    widgets::traffic_panel(store, ui);

    let mut explore = store.preferences.explore_multihop == Some(true);
    if ui
        .add_enabled(
            !store.running && !store.busy && !store.is_testing_latency,
            egui::Checkbox::new(&mut explore, "探索多跳链路（高级）"),
        )
        .changed()
    {
        store.preferences.explore_multihop = Some(explore);
        store.save_preferences();
    }
    hint(ui, "出口切换时，已有 TCP 连接可能需要重连。");
}

fn arrow(ui: &mut Ui) {
    ui.label(
        RichText::new("→")
            .size(16.0)
            .color(widgets::ACCENT),
    );
}

fn hop(ui: &mut Ui, title: &str, detail: &str, relay: bool) {
    let (fill, stroke) = if relay {
        (widgets::SURFACE_ALT, Stroke::new(1.0, widgets::ACCENT))
    } else {
        (widgets::SURFACE_ALT, Stroke::new(1.0, widgets::BORDER))
    };
    egui::Frame::default()
        .fill(fill)
        .stroke(stroke)
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::same(10.0))
        .show(ui, |ui| {
            ui.set_width(150.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(title).strong().size(13.5).color(widgets::TEXT));
                ui.label(RichText::new(detail).size(11.0).color(MUTED));
            });
        });
}
