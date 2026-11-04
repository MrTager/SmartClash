use super::widgets::{self, hint};
use crate::state::Store;
use egui::{RichText, Ui};

pub fn draw(store: &Store, ui: &mut Ui) {
    widgets::page_title(
        ui,
        "连接事件",
        "只显示连接与切换结果，不显示订阅 URL 或节点密码。内核诊断文件在本机保存。",
    );

    widgets::card(ui, None, |ui| {
        if store.logs.is_empty() {
            hint(ui, "暂无事件");
        } else {
            egui::ScrollArea::vertical()
                .max_height(460.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for line in &store.logs {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.label(
                                RichText::new("●")
                                    .size(7.0)
                                    .color(widgets::MUTED),
                            );
                            ui.label(
                                RichText::new(line)
                                    .monospace()
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(188, 196, 210)),
                            );
                        });
                        ui.add_space(1.0);
                    }
                });
        }
    });

    ui.add_space(6.0);
    if widgets::accent_button(ui, "打开诊断目录", true) {
        let root = crate::services::paths::data_root();
        let _ = std::fs::create_dir_all(&root);
        crate::services::paths::open_in_explorer(&root);
    }
    ui.add_space(2.0);
    hint(ui, "诊断目录包含配置、订阅缓存与内核日志，可能含节点凭据，请按敏感文件处理。");
}
