#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_icon;
mod logic;
mod services;
mod state;
mod ui;

use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1080.0, 780.0])
            .with_min_inner_size([940.0, 650.0])
            .with_title("SmartClash")
            .with_icon(egui::IconData {
                rgba: app_icon::RGBA.to_vec(),
                width: app_icon::SIDE,
                height: app_icon::SIDE,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "SmartClash",
        options,
        Box::new(|cc| {
            configure_style(&cc.egui_ctx);
            install_fonts(&cc.egui_ctx);
            let store = state::Store::new()
                .map_err(|error| Box::<dyn std::error::Error + Send + Sync>::from(error))?;
            let shared = Arc::new(Mutex::new(store));
            shared.lock().unwrap().egui_ctx = Some(cc.egui_ctx.clone());
            Ok(Box::new(App {
                shared,
                view: ui::ViewState::default(),
            }))
        }),
    )
}

struct App {
    shared: state::Shared,
    view: ui::ViewState,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut pending: Vec<ui::Action> = Vec::new();
        {
            let mut store = self.shared.lock().unwrap();
            ui::draw(&mut self.view, &self.shared, &mut store, ctx, &mut pending);
        }
        for action in pending {
            dispatch(action, &self.shared, ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        state::actions::shutdown(&self.shared);
    }
}

fn dispatch(action: ui::Action, shared: &state::Shared, ctx: &egui::Context) {
    use state::actions;
    use ui::Action::*;
    match action {
        Refresh => actions::refresh(shared, ctx),
        StartOnly => actions::start_only(shared, ctx),
        Stop => actions::stop_all(shared, ctx),
        ToggleSystem => actions::toggle_system(shared, ctx),
        ToggleTun => actions::toggle_tun(shared, ctx),
        Connect => actions::connect(shared, ctx),
        ApplyMode => actions::apply_mode(shared, ctx),
        TrafficMode(mode) => actions::change_traffic_mode(shared, ctx, mode),
        LatencyAll => actions::begin_latency_all(shared, ctx),
        LatencyOne(node) => actions::begin_latency_one(shared, ctx, node),
        SelectVerified(node) => actions::select_verified(shared, ctx, node),
        CancelLatency => actions::cancel_latency(shared),
        InspectNow => actions::inspect_now(shared, ctx),
        UpdateRules => actions::update_rules(shared, ctx),
        Kernel(kind) => actions::change_kernel(shared, ctx, kind),
        ProxyPort(port) => actions::change_proxy_port(shared, ctx, port),
        AddSource(name, url) => {
            if let Err(error) = actions::add_source(shared, &name, &url) {
                let mut store = shared.lock().unwrap();
                store.error = Some(error);
            }
        }
        RemoveSource(id) => actions::remove_source(shared, &id),
        Export => actions::export_yaml(shared),
    }
}

fn configure_style(ctx: &egui::Context) {
    use egui::{Color32, Rounding, Stroke, Vec2};

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(13, 15, 21);
    visuals.window_fill = Color32::from_rgb(24, 27, 36);
    visuals.faint_bg_color = Color32::from_rgb(20, 23, 31);
    visuals.extreme_bg_color = Color32::from_rgb(11, 13, 18);
    visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(48, 54, 70));
    visuals.selection.bg_fill = Color32::from_rgb(52, 60, 96);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(108, 140, 255));
    visuals.hyperlink_color = ui::widgets::ACCENT;

    // 控件圆角
    let rounding = Rounding::same(7.0);
    visuals.widgets.noninteractive.rounding = rounding;
    visuals.widgets.inactive.rounding = rounding;
    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.active.rounding = rounding;
    visuals.widgets.open.rounding = rounding;
    visuals.window_rounding = Rounding::same(12.0);
    visuals.menu_rounding = Rounding::same(8.0);

    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(44, 49, 64));
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(150, 158, 174));
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(205, 211, 222));
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 38, 50);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(48, 54, 68));
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 47, 62);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(62, 70, 92));
    visuals.widgets.active.bg_fill = Color32::from_rgb(48, 54, 72);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(72, 82, 108));

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.text_edit_width = 160.0;

    // 全局字体层级：整体更舒展
    use std::collections::BTreeMap;
    let mut text_styles = BTreeMap::new();
    text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(20.0));
    text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
    style.text_styles = text_styles;

    ctx.set_style(style);
}

fn install_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\Deng.ttf",
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\SimsunExtG.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
    ];
    let Some(bytes) = candidates
        .iter()
        .find_map(|path| std::fs::read(path).ok())
    else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("cjk".to_owned(), egui::FontData::from_owned(bytes).into());
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "cjk".to_owned());
    }
    ctx.set_fonts(fonts);
}
