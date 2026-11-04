use super::widgets::{self, hint, BAD, BAD_DIM, GOOD, GOOD_DIM, MUTED, WARN, WARN_DIM};
use super::{Action, ViewState};
use crate::logic::model::NodeSortOrder;
use crate::state::Store;
use egui::{Color32, RichText, Ui};

pub fn draw(
    view: &mut ViewState,
    store: &mut Store,
    ui: &mut Ui,
    pending: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("节点与延迟").size(24.0).strong().color(widgets::TEXT));
            hint(ui, "测试 HTTPS 出境延迟；结果只读，不改变当前业务出口。");
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut view.node_query)
                    .hint_text("🔍 搜索节点")
                    .desired_width(220.0),
            );
        });
    });
    ui.add_space(10.0);

    let nodes = store.node_list_nodes();
    let query = view.node_query.trim().to_lowercase();
    let mut indexes: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| query.is_empty() || node.name.to_lowercase().contains(&query))
        .map(|(index, _)| index)
        .collect();
    match view.node_sort {
        NodeSortOrder::Original => {}
        NodeSortOrder::Name => {
            indexes.sort_by(|a, b| nodes[*a].name.to_lowercase().cmp(&nodes[*b].name.to_lowercase()))
        }
        NodeSortOrder::Latency => indexes.sort_by(|a, b| {
            let left = store.latencies.get(&nodes[*a].id);
            let right = store.latencies.get(&nodes[*b].id);
            let rank = |value: Option<&crate::logic::model::NodeLatency>| match value {
                Some(v) if v.reachable() => 0,
                None => 1,
                _ => 2,
            };
            let (l, r) = (rank(left), rank(right));
            if l != r {
                return l.cmp(&r);
            }
            let lm = left.and_then(|v| v.milliseconds).unwrap_or(i64::MAX);
            let rm = right.and_then(|v| v.milliseconds).unwrap_or(i64::MAX);
            if lm != rm {
                return lm.cmp(&rm);
            }
            nodes[*a]
                .name
                .to_lowercase()
                .cmp(&nodes[*b].name.to_lowercase())
        }),
    }

    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("node-sort")
            .selected_text(view.node_sort.title())
            .show_ui(ui, |ui| {
                for order in NodeSortOrder::ALL {
                    ui.selectable_value(&mut view.node_sort, order, order.title());
                }
            });

        widgets::status_pill(
            ui,
            &format!("{} 个节点", indexes.len()),
            widgets::TEXT,
            widgets::MUTED_DIM,
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if store.is_testing_latency {
                ui.label(
                    RichText::new(format!(
                        "{}/{}",
                        store.latency_completed, store.latency_total
                    ))
                    .monospace(),
                );
                if ui.button("取消测速").clicked() {
                    pending.push(Action::CancelLatency);
                }
            } else if widgets::accent_button(
                ui,
                "全部测速",
                !store.busy && !store.recovery && !store.nodes.is_empty(),
            ) {
                pending.push(Action::LatencyAll);
            }
        });
    });

    ui.add_space(4.0);
    if store.is_testing_latency {
        let fraction = store.latency_completed as f32 / (store.latency_total.max(1)) as f32;
        ui.add(egui::ProgressBar::new(fraction).show_percentage());
    }
    ui.add_space(2.0);
    hint(
        ui,
        "测试节点完成 HTTPS 请求的延迟，6 路并发、单次最多 5 秒；不会切换当前业务出口或开启系统代理 / TUN。",
    );
    ui.add_space(8.0);

    widgets::card(ui, None, |ui| {
        if indexes.is_empty() {
            hint(ui, "没有匹配的节点");
            return;
        }
        let row_height = 46.0;
        egui::ScrollArea::vertical()
            .max_height(420.0)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, indexes.len(), |ui, range| {
                let range_end = range.end;
                for row in range {
                    let node = &nodes[indexes[row]];
                    let selected = node.name == store.selected;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(if selected { "●" } else { "○" })
                                .size(11.0)
                                .color(if selected { GOOD } else { MUTED }),
                        );
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&node.name).strong().color(widgets::TEXT));
                            let source_name = store
                                .preferences
                                .sources
                                .iter()
                                .find(|s| s.id == node.source_id)
                                .map(|s| s.name.clone())
                                .unwrap_or_else(|| "动态链路".into());
                            let extra = store
                                .latencies
                                .get(&node.id)
                                .map(|l| crate::services::paths::datetime_text(l.checked))
                                .unwrap_or_default();
                            hint(ui, &format!("{source_name}  {extra}"));
                        });
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                let can_select = store.running
                                    && !store.busy
                                    && !store.is_testing_latency
                                    && !store.stopping;
                                if ui
                                    .add_enabled(
                                        can_select,
                                        egui::Button::new(
                                            RichText::new("选择").size(13.0).color(widgets::ACCENT),
                                        )
                                        .stroke(egui::Stroke::new(1.0, widgets::ACCENT)),
                                    )
                                    .clicked()
                                {
                                    pending.push(Action::SelectVerified(node.clone()));
                                }
                                if ui
                                    .add_enabled(
                                        !store.busy
                                            && !store.is_testing_latency
                                            && !store.recovery,
                                        egui::Button::new(
                                            RichText::new("测速").size(13.0),
                                        ),
                                    )
                                    .clicked()
                                {
                                    pending.push(Action::LatencyOne(node.clone()));
                                }
                                ui.add_space(4.0);
                                latency_label(ui, store, &node.id);
                            },
                        );
                    });
                    if row + 1 < range_end {
                        ui.separator();
                    }
                }
            });
    });

    ui.add_space(6.0);
    hint(
        ui,
        "可达节点按延迟从低到高排序，未测与失败节点靠后。延迟不是下载速度，也不代表 OpenAI / ChatGPT 的地区资格。",
    );
}

fn latency_label(ui: &mut Ui, store: &Store, id: &str) {
    if store.testing_ids.contains(id) {
        widgets::status_pill(ui, "测试中", MUTED, widgets::MUTED_DIM);
        return;
    }
    match store.latencies.get(id) {
        Some(sample) => {
            if let Some(ms) = sample.milliseconds {
                if sample.reachable() {
                    if ms < 1000 {
                        widgets::status_pill(ui, &format!("{ms} ms"), GOOD, GOOD_DIM);
                    } else {
                        widgets::status_pill(ui, &format!("{ms} ms"), WARN, WARN_DIM);
                    }
                } else {
                    widgets::status_pill(ui, "不可达", BAD, BAD_DIM);
                }
            } else {
                let failure = sample.failure.clone().unwrap_or_else(|| "不可达".into());
                widgets::status_pill(ui, &failure, BAD, BAD_DIM);
            }
        }
        None => widgets::status_pill(ui, "未测速", MUTED, widgets::MUTED_DIM),
    }
}
