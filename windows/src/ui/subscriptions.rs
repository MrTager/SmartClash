use super::widgets::{self, hint, MUTED, TEXT};
use super::{Action, ViewState};
use crate::logic::model::AGGREGATE_PORT;
use crate::state::Store;
use egui::{Color32, RichText, Ui};

pub fn draw(
    view: &mut ViewState,
    store: &mut Store,
    ui: &mut Ui,
    pending: &mut Vec<Action>,
) {
    widgets::page_title(
        ui,
        "多个订阅，一个入口",
        "导入 Clash YAML / JSON 订阅，按节点配置去重。更新失败保留缓存。订阅需手动更新；TUN 模式请停止后更新。",
    );

    widgets::card(ui, Some("添加订阅"), |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut view.source_name)
                    .hint_text("名称")
                    .desired_width(140.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut view.source_url)
                    .hint_text("HTTPS 订阅链接")
                    .password(true)
                    .desired_width(ui.available_width() - 100.0),
            );
            let enabled = store.can_change_sources() && !view.source_url.trim().is_empty();
            if widgets::accent_button(ui, "添加", enabled) {
                pending.push(Action::AddSource(
                    view.source_name.clone(),
                    view.source_url.clone(),
                ));
                view.source_name.clear();
                view.source_url.clear();
            }
        });
    });
    hint(
        ui,
        "添加或删除订阅前请停止内核。不支持仅含 proxy-providers 的配置、Base64 节点链接及节点链。",
    );
    ui.add_space(8.0);

    widgets::card(ui, Some("订阅来源"), |ui| {
        if store.preferences.sources.is_empty() {
            hint(ui, "尚未添加订阅");
        } else {
            let sources = store.preferences.sources.clone();
            let can_change = store.can_change_sources();
            for source in sources {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("●")
                            .size(10.0)
                            .color(widgets::ACCENT),
                    );
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&source.name).strong().color(TEXT));
                        hint(
                            ui,
                            if source.is_local() {
                                "本地配置 · 离线快照"
                            } else {
                                source.url.as_str()
                            },
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                can_change,
                                egui::Button::new(
                                    RichText::new("删除").size(13.0).color(widgets::BAD),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(egui::Stroke::new(1.0, widgets::BAD)),
                            )
                            .clicked()
                        {
                            pending.push(Action::RemoveSource(source.id.clone()));
                        }
                    });
                });
                ui.separator();
            }
        }
    });
    ui.add_space(4.0);

    widgets::card(ui, Some("聚合输出"), |ui| {
        ui.label(RichText::new("本机订阅链接（含随机访问令牌）").strong().color(TEXT));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if store.subscription_ready {
                    format!(
                        "http://127.0.0.1:{AGGREGATE_PORT}/{}…/config.yaml",
                        &store.preferences.subscription_token[..6.min(store.preferences.subscription_token.len())]
                    )
                } else {
                    "订阅服务不可用".to_string()
                })
                .monospace()
                .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ready = store.subscription_ready && !store.nodes.is_empty();
                if widgets::accent_button(ui, "复制订阅链接", ready) {
                    ui.ctx().copy_text(store.subscription_url());
                }
                if ui
                    .add_enabled(!store.nodes.is_empty(), egui::Button::new("导出 YAML"))
                    .clicked()
                {
                    pending.push(Action::Export);
                }
            });
        });
        ui.add_space(4.0);
        hint(
            ui,
            "链接仅当前 Windows 本机可访问，客户端需保持本应用运行。导出内容含节点密码，请妥善保管。其他客户端应选择“直连”更新本机订阅。",
        );
    });
}
