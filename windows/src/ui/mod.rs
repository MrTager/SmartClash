use crate::logic::model::NodeSortOrder;
use crate::logic::model::ProxyNode;
use crate::logic::model::TrafficMode;
use crate::state::Shared;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Overview,
    Subscriptions,
    Chain,
    Nodes,
    Logs,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Overview,
        Section::Subscriptions,
        Section::Chain,
        Section::Nodes,
        Section::Logs,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Overview => "概览",
            Section::Subscriptions => "订阅聚合",
            Section::Chain => "链式代理",
            Section::Nodes => "节点",
            Section::Logs => "事件日志",
        }
    }
}

pub struct ViewState {
    pub section: Section,
    pub source_name: String,
    pub source_url: String,
    pub node_query: String,
    pub node_sort: NodeSortOrder,
    pub chain_entry_ids: std::collections::BTreeSet<String>,
    pub chain_exit_ids: std::collections::BTreeSet<String>,
    pub show_chain_editor: bool,
    pub globe_yaw: f32,
    pub globe_pitch: f32,
    pub globe_auto: bool,
    pub globe_texture: Option<egui::TextureHandle>,
    pub globe_texture_key: (f32, f32, u32),
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            section: Section::Overview,
            source_name: String::new(),
            source_url: String::new(),
            node_query: String::new(),
            node_sort: NodeSortOrder::Latency,
            chain_entry_ids: Default::default(),
            chain_exit_ids: Default::default(),
            show_chain_editor: false,
            globe_yaw: 105.0,
            globe_pitch: 20.0,
            globe_auto: false,
            globe_texture: None,
            globe_texture_key: (f32::NAN, f32::NAN, 0),
        }
    }
}

pub enum Action {
    Refresh,
    StartOnly,
    Stop,
    ToggleSystem,
    ToggleTun,
    Connect,
    ApplyMode,
    TrafficMode(TrafficMode),
    LatencyAll,
    LatencyOne(ProxyNode),
    SelectVerified(ProxyNode),
    CancelLatency,
    InspectNow,
    AddSource(String, String),
    RemoveSource(String),
    Export,
}

pub fn draw(
    view: &mut ViewState,
    _shared: &Shared,
    store: &mut crate::state::Store,
    ctx: &egui::Context,
    pending: &mut Vec<Action>,
) {
    egui::SidePanel::left("navigation")
        .resizable(false)
        .exact_width(200.0)
        .frame(
            egui::Frame::default()
                .fill(widgets::SIDEBAR_BG)
                .inner_margin(egui::Margin::symmetric(14.0, 16.0)),
        )
        .show(ctx, |ui| {
            // 品牌区
            ui.horizontal(|ui| {
                egui::Frame::default()
                    .fill(widgets::ACCENT_SOFT)
                    .rounding(egui::Rounding::same(9.0))
                    .inner_margin(egui::Margin::symmetric(11.0, 7.0))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("S")
                                .size(17.0)
                                .strong()
                                .color(widgets::ACCENT),
                        );
                    });
                ui.label(egui::RichText::new("SmartClash").size(19.0).strong());
            });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Windows · Rust")
                    .size(11.0)
                    .color(widgets::MUTED),
            );
            ui.add_space(18.0);

            for section in Section::ALL {
                if widgets::nav_item(ui, section.title(), view.section == section) {
                    view.section = section;
                }
                ui.add_space(3.0);
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(10.0);
                let (text, color) = if store.running {
                    ("代理运行中", widgets::GOOD)
                } else if store.recovery {
                    ("需要恢复网络", widgets::WARN)
                } else {
                    ("代理已停止", widgets::MUTED)
                };
                egui::Frame::default()
                    .fill(widgets::SURFACE)
                    .stroke(egui::Stroke::new(1.0, widgets::BORDER))
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 7.0;
                            ui.label(egui::RichText::new("●").size(9.0).color(color));
                            ui.label(
                                egui::RichText::new(text)
                                    .size(12.0)
                                    .color(color),
                            );
                        });
                    });
                ui.add_space(8.0);
                widgets::hint(ui, "本地处理 · 私有订阅");
            });
        });

    egui::TopBottomPanel::top("toolbar")
        .frame(
            egui::Frame::default()
                .fill(egui::Color32::from_rgb(13, 15, 21))
                .inner_margin(egui::Margin::symmetric(20.0, 10.0)),
        )
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(view.section.title())
                            .size(16.0)
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_refresh = !store.busy
                            && !store.is_testing_latency
                            && !store.tun
                            && !store.preferences.sources.is_empty();
                        if widgets::accent_button(ui, "更新订阅", can_refresh) {
                            pending.push(Action::Refresh);
                        }
                        if store.busy {
                            ui.add_space(6.0);
                            ui.spinner();
                        }
                    });
                });
                ui.add_space(6.0);
                let line = ui.allocate_response(
                    egui::vec2(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(line.rect, egui::Rounding::same(0.0), widgets::BORDER);
            });
        });

    egui::CentralPanel::default()
        .frame(
            egui::Frame::default()
                .fill(egui::Color32::from_rgb(13, 15, 21))
                .inner_margin(egui::Margin::symmetric(20.0, 4.0)),
        )
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    match view.section {
                        Section::Overview => overview::draw(view, store, ui, pending),
                        Section::Subscriptions => subscriptions::draw(view, store, ui, pending),
                        Section::Chain => chain::draw(view, store, ui, ctx, pending),
                        Section::Nodes => nodes::draw(view, store, ui, pending),
                        Section::Logs => logs::draw(store, ui),
                    }
                    ui.add_space(20.0);
                });
        });

    if let Some(error) = store.error.clone() {
        let mut open = true;
        egui::Window::new("操作提示")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(
                        egui::RichText::new("●").size(10.0).color(widgets::WARN),
                    );
                    ui.label(egui::RichText::new(error).size(14.0));
                });
                ui.add_space(10.0);
                ui.set_min_width(240.0);
                ui.vertical_centered(|ui| {
                    if widgets::primary_button(
                        ui,
                        "好",
                        widgets::ACCENT,
                        true,
                        egui::vec2(96.0, 32.0),
                    ) {
                        store.error = None;
                    }
                });
            });
        if !open {
            store.error = None;
        }
    }
}

pub mod chain;
pub mod globe;
pub mod logs;
pub mod nodes;
pub mod overview;
pub mod subscriptions;
pub mod widgets;
