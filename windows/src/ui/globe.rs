//! 全球连接地球面板：与 macOS 版 NetworkGlobePanel + GlobeScene 1:1 对齐。
//! 数据来自真实的节点服务器 IP 与内核实时连接（ipwho.is 定位，24h 缓存）。

use super::widgets::{self, hint};
use super::ViewState;
use crate::logic::chain::ChainTrace;
use crate::logic::model::{CoreConnection, GeoLocation, ProxyNode};
use crate::state::Store;
use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::HashMap;
use std::sync::OnceLock;

const LAND: &str = include_str!("../../resources/world-land.json");

// ---- 配色（对应 GlobeScene.swift / earthTexture()） ------------------------
const SCENE_BG: Color32 = Color32::from_rgb(6, 11, 20); // 0.025, 0.045, 0.08
const OCEAN: [f32; 3] = [6.0, 19.0, 33.0]; // 0.025, 0.075, 0.13
const LAND_FILL: [f32; 3] = [26.0, 74.0, 89.0]; // 0.10, 0.29, 0.35
const LAND_STROKE: [f32; 3] = [46.0, 130.0, 143.0]; // 0.18, 0.51, 0.56
const CYAN: Color32 = Color32::from_rgb(0, 255, 255);
const TEAL: Color32 = Color32::from_rgb(100, 210, 255);
const ORANGE: Color32 = Color32::from_rgb(255, 159, 10);
// 半透明白：from_rgba_unmultiplied 非 const，用函数近似。
fn white(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, alpha)
}

const DEFAULT_LAT: f32 = 23.0;
const DEFAULT_LON: f32 = 110.0;
const MIN_DISTANCE: f32 = 3.0;
const MAX_DISTANCE: f32 = 8.0;
const DEFAULT_DISTANCE: f32 = 3.9;

// ---- 快照（对应 GlobeSnapshot.swift） --------------------------------------

#[derive(Clone)]
struct Marker {
    id: String,
    lat: f32,
    lon: f32,
    is_node: bool,
    active: bool,
}

#[derive(Clone)]
struct Route {
    locations: Vec<(f32, f32)>,
    selected: bool,
    highlighted: bool,
}

struct Snapshot {
    markers: Vec<Marker>,
    routes: Vec<Route>,
    mapped_nodes: usize,
    mapped_connections: usize,
    selected_node_id: Option<String>,
    selected_location: Option<(f32, f32)>,
}

fn node_server(node: &ProxyNode) -> Option<String> {
    node.payload
        .get("server")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn build_snapshot(
    nodes: &[ProxyNode],
    trace: &ChainTrace,
    connections: &[CoreConnection],
    locations: &HashMap<String, GeoLocation>,
    focused: Option<&str>,
) -> Snapshot {
    let mut markers: Vec<Marker> = Vec::new();
    let active_ids: std::collections::HashSet<&String> =
        trace.hops.iter().map(|hop| &hop.id).collect();

    let insert_marker = |markers: &mut Vec<Marker>, marker: Marker| {
        if let Some(existing) = markers.iter_mut().find(|m| m.id == marker.id) {
            if marker.active {
                *existing = marker;
            }
        } else {
            markers.push(marker);
        }
    };

    // 精确跟随完整链路末跳；位置未知时不回退入口或旧的活跃标记。
    let selected_node = trace
        .complete
        .then(|| trace.hops.last())
        .flatten()
        .and_then(|hop| nodes.iter().find(|node| node.id == hop.id));
    let selected_node_id = selected_node.map(|node| node.id.clone());
    let selected_location = selected_node
        .and_then(node_server)
        .and_then(|server| locations.get(&server))
        .filter(|location| location.is_valid())
        .map(|location| (location.latitude as f32, location.longitude as f32));

    for node in nodes {
        let Some(server) = node_server(node) else {
            continue;
        };
        let Some(location) = locations.get(&server) else {
            continue;
        };
        insert_marker(
            &mut markers,
            Marker {
                id: format!("node:{server}"),
                lat: location.latitude as f32,
                lon: location.longitude as f32,
                is_node: true,
                active: active_ids.contains(&node.id),
            },
        );
    }
    let mapped_nodes = markers.len();

    let index: HashMap<String, &ProxyNode> =
        nodes.iter().map(|node| (node.name.clone(), node)).collect();

    let mut routes: Vec<Route> = Vec::new();
    let mut mapped_connections = 0usize;

    // 当前链路（选中出口的完整路径）。
    let current: Vec<(f32, f32)> = trace
        .hops
        .iter()
        .filter_map(|hop| {
            let server = nodes
                .iter()
                .find(|node| node.id == hop.id)
                .and_then(|node| node_server(node))?;
            locations
                .get(&server)
                .map(|l| (l.latitude as f32, l.longitude as f32))
        })
        .collect();
    if current.len() > 1 && current.len() == trace.hops.len() && trace.complete {
        routes.push(Route {
            locations: current,
            selected: true,
            highlighted: true,
        });
    }

    // 实时连接：聚焦的排最前（对应 macOS 的排序）。
    let mut ordered: Vec<&CoreConnection> = connections.iter().collect();
    ordered.sort_by_key(|c| if Some(c.id.as_str()) == focused { 0 } else { 1 });
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for connection in ordered {
        let Some(key) = connection.metadata.geolocation_key() else {
            continue;
        };
        let Some(destination) = locations.get(&key) else {
            continue;
        };
        let path = connection.route_nodes(&index);
        if path.is_empty() {
            continue;
        }
        let mut hops: Vec<(f32, f32)> = Vec::new();
        for node in &path {
            let Some(server) = node_server(node) else {
                break;
            };
            match locations.get(&server) {
                Some(l) => hops.push((l.latitude as f32, l.longitude as f32)),
                None => break,
            }
        }
        if hops.len() != path.len() {
            continue;
        }
        mapped_connections += 1;
        insert_marker(
            &mut markers,
            Marker {
                id: format!("target:{key}"),
                lat: destination.latitude as f32,
                lon: destination.longitude as f32,
                is_node: false,
                active: Some(connection.id.as_str()) == focused,
            },
        );
        hops.push((destination.latitude as f32, destination.longitude as f32));
        let route_key = format!(
            "{}:{key}",
            path.iter()
                .map(|n| n.id.clone())
                .collect::<Vec<_>>()
                .join("/")
        );
        if seen.insert(route_key) && routes.len() < 48 {
            routes.push(Route {
                locations: hops,
                selected: false,
                highlighted: focused.is_none() || Some(connection.id.as_str()) == focused,
            });
        }
    }

    markers.sort_by(|a, b| a.id.cmp(&b.id));
    Snapshot {
        markers,
        routes,
        mapped_nodes,
        mapped_connections,
        selected_node_id,
        selected_location,
    }
}

/// 仅出口身份/位置改变时跟随；连接轮询不能覆盖用户的手动视角。
fn follow_selection(view: &mut ViewState, snapshot: &Snapshot, trace: &ChainTrace) {
    let route_ids: Vec<String> = trace.hops.iter().map(|hop| hop.id.clone()).collect();
    if view.globe_route_ids != route_ids {
        view.focused_connection = None;
        view.globe_route_ids = route_ids;
    }
    let changed = view.globe_selected_node != snapshot.selected_node_id
        || view.globe_selected_location != snapshot.selected_location;
    view.globe_selected_node = snapshot.selected_node_id.clone();
    view.globe_selected_location = snapshot.selected_location;
    if changed {
        if let Some(location) = snapshot.selected_location {
            reset_camera(view, Some(location));
        }
    }
}

// ---- 主绘制 ---------------------------------------------------------------

pub fn draw(store: &Store, ui: &mut Ui, view: &mut ViewState) {
    let activity = store
        .activity
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let route_ids: Vec<String> = store
        .active_trace
        .hops
        .iter()
        .map(|hop| hop.id.clone())
        .collect();
    if view.globe_route_ids != route_ids
        || view
            .focused_connection
            .as_ref()
            .is_some_and(|id| !activity.connections.iter().any(|c| &c.id == id))
    {
        view.focused_connection = None;
    }
    let nodes = store.visualization_nodes();
    let snapshot = build_snapshot(
        &nodes,
        &store.active_trace,
        &activity.connections,
        &activity.locations,
        view.focused_connection.as_deref(),
    );
    follow_selection(view, &snapshot, &store.active_trace);

    widgets::card(ui, None, |ui| {
        // ---- 头部：全球连接 + 实时状态（对应 NetworkGlobePanel 头部） ----
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("● 全球连接")
                    .size(15.0)
                    .strong()
                    .color(widgets::TEXT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (color, text) = if !activity.supported {
                    (widgets::MUTED, "当前内核不提供实时连接".to_string())
                } else if activity.connected {
                    (
                        widgets::GOOD,
                        format!("实时 · {} 个连接", activity.connections.len()),
                    )
                } else if store.running {
                    (widgets::MUTED, "正在连接内核…".to_string())
                } else {
                    (widgets::MUTED, "等待连接".to_string())
                };
                ui.label(RichText::new("●").size(8.0).color(color));
                ui.label(RichText::new(text).size(12.0).color(widgets::MUTED));
            });
        });
        ui.add_space(12.0);

        // ---- 深色地球区域 ----
        let width = ui.available_width().max(280.0);
        let height = 330.0_f32.min(width * 0.62).max(250.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());

        // 拖动旋转（0.008 rad/px，与 macOS rotateCamera 一致）。
        // egui 的 delta.y 向下为正，向下拖 = 相机纬度升高。
        if response.dragged() {
            let delta = response.drag_delta();
            view.globe_yaw -= delta.x * 0.458;
            view.globe_pitch = (view.globe_pitch + delta.y * 0.458).clamp(-85.0, 85.0);
        }
        // 滚轮缩放（3–8，与 macOS zoomCamera 一致）。
        if response.hovered() {
            let scroll = ui.input(|input| input.raw_scroll_delta.y);
            if scroll != 0.0 {
                view.globe_distance = (view.globe_distance * (-scroll * 0.0012).exp())
                    .clamp(MIN_DISTANCE, MAX_DISTANCE);
            }
        }
        if view.globe_yaw > 180.0 {
            view.globe_yaw -= 360.0;
        } else if view.globe_yaw < -180.0 {
            view.globe_yaw += 360.0;
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, egui::Rounding::same(12.0), SCENE_BG);
        painter.rect_stroke(
            rect,
            egui::Rounding::same(12.0),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 20)),
        );

        let center = rect.center();
        // FOV 42°、距离 3.9 时球体直径约占视口高度 71%。
        let base = (rect.height() * 0.354).min(rect.width() * 0.40);
        let radius = base * DEFAULT_DISTANCE / view.globe_distance;
        let yaw = view.globe_yaw;
        let pitch = view.globe_pitch;
        let now = ui.input(|input| input.time) as f32;

        // ---- 球体纹理 ----
        let pixels = (radius * 2.8).clamp(256.0, 512.0) as usize;
        let texture = globe_texture(ui, view, yaw, pitch, pixels);
        painter.image(
            texture.id(),
            Rect::from_center_size(center, Vec2::splat(radius * 2.0)),
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        // ---- 航线（对应 tube + tracer 粒子） ----
        for route in &snapshot.routes {
            let color = if route.selected { CYAN } else { ORANGE };
            let alpha = if route.highlighted { 242 } else { 51 };
            let stroke_color =
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
            let width = if route.selected {
                (radius * 0.008).max(2.0)
            } else {
                (radius * 0.005).max(1.1)
            };
            let duration = if route.selected { 3.0 } else { 2.4 };
            for window in route.locations.windows(2) {
                let points = arc_points(window[0], window[1]);
                let mut previous: Option<(Pos2, f32)> = None;
                for point in &points {
                    let lifted = (point.0, point.1, point.2);
                    let (position, visible) = project_vec(lifted, center, radius, pitch, yaw);
                    if let Some((last, last_visible)) = previous {
                        if visible > 0.02 && last_visible > 0.02 {
                            painter
                                .line_segment([last, position], Stroke::new(width, stroke_color));
                        }
                    }
                    previous = Some((position, visible));
                }
                if route.highlighted {
                    let progress = (now / duration).fract();
                    let index = (progress * (points.len() - 1) as f32).round() as usize;
                    let point = points[index.min(points.len() - 1)];
                    let (position, visible) = project_vec(point, center, radius, pitch, yaw);
                    if visible > 0.02 {
                        let size = (radius * 0.007).max(1.5);
                        painter.circle_filled(position, size, color);
                    }
                }
            }
        }

        // ---- 标记（节点青色 / 目标橙色，激活带光环） ----
        for marker in &snapshot.markers {
            let scale = if marker.is_node { 1.012 } else { 1.012 };
            let (x, y, z) = unit_vector(marker.lat, marker.lon);
            let (position, visible) = project_vec(
                (x * scale, y * scale, z * scale),
                center,
                radius,
                pitch,
                yaw,
            );
            if visible <= 0.02 {
                continue;
            }
            let color = if marker.is_node {
                if marker.active {
                    CYAN
                } else {
                    TEAL
                }
            } else {
                ORANGE
            };
            let size = (radius * if marker.active { 0.018 } else { 0.010 }).max(1.5);
            if marker.active {
                let halo = (radius * 0.032).max(size + 2.0);
                painter.circle_stroke(
                    position,
                    halo,
                    Stroke::new(
                        (radius * 0.004).max(1.0),
                        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 178),
                    ),
                );
            }
            painter.circle_filled(position, size, color);
        }

        // ---- 覆盖层文字（对应 ZStack 内 VStack） ----
        let pad = 16.0;
        let mut cursor = Pos2::new(rect.min.x + pad, rect.min.y + 14.0);
        let trace_text = if store.active_trace.hops.is_empty() {
            "尚未选择出口".to_string()
        } else {
            format!("当前 {} 跳线路", store.active_trace.hops.len())
        };
        cursor = paint_line(&painter, cursor, &trace_text, FontId::monospace(12.0), CYAN);
        if let Some(last) = store.active_trace.hops.last() {
            let title = last.name.split(" · ").next().unwrap_or(&last.name);
            cursor = paint_line(
                &painter,
                cursor,
                title,
                FontId::proportional(15.0),
                Color32::WHITE,
            );
        }
        cursor = paint_line(
            &painter,
            cursor,
            &format!(
                "{} 处节点 · {} 个请求已定位",
                snapshot.mapped_nodes, snapshot.mapped_connections
            ),
            FontId::proportional(12.0),
            white(166),
        );
        if !store.active_trace.hops.is_empty() && snapshot.selected_location.is_none() {
            paint_line(
                &painter,
                cursor,
                "当前节点位置待解析",
                FontId::proportional(10.5),
                white(166),
            );
        } else if activity.resolving {
            paint_line(
                &painter,
                cursor,
                "正在解析 IP 位置…",
                FontId::proportional(10.5),
                white(128),
            );
        }

        // 底部图例。
        let legend_y = rect.max.y - 26.0;
        let mut legend_x = rect.min.x + pad;
        legend_x = paint_legend(&painter, Pos2::new(legend_x, legend_y), CYAN, "代理节点") + 16.0;
        paint_legend(
            &painter,
            Pos2::new(legend_x, legend_y),
            ORANGE,
            "请求目标 / 流向",
        );

        // 未连接时底部居中提示。
        if !activity.enabled {
            let overlay = if activity.supported {
                "连接后显示节点与请求目标的真实连线"
            } else {
                "当前内核不提供实时连接视图"
            };
            let galley =
                painter.layout_no_wrap(overlay.into(), FontId::proportional(12.0), white(166));
            let start = Pos2::new(
                center.x - galley.size().x / 2.0,
                rect.max.y - 49.0 - galley.size().y,
            );
            painter.galley(start, galley, white(166));
        }

        // 有动画元素时持续重绘。
        if !snapshot.routes.is_empty() || activity.resolving {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(60));
        }

        // 右下角“回到当前节点”按钮。
        let button_size = egui::vec2(110.0, 22.0);
        let button_rect = Rect::from_min_size(
            Pos2::new(
                rect.max.x - pad - button_size.x,
                rect.max.y - 12.0 - button_size.y,
            ),
            button_size,
        );
        let reset = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(button_rect), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled(
                    snapshot.selected_location.is_some(),
                    egui::Button::new(RichText::new("◎ 回到当前节点").size(11.5).color(white(217)))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE),
                )
                .clicked()
            })
            .inner
        });
        if reset.inner {
            reset_camera(view, snapshot.selected_location);
        }
    });

    // ---- 底部操作行 ----
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        hint(
            ui,
            "拖动旋转 · 滚轮缩放 · 节点服务器 IP 为近似位置，非实际出网 IP",
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new(
                    RichText::new(if view.show_connections {
                        "收起请求"
                    } else {
                        "查看实时请求"
                    })
                    .size(12.0)
                    .color(widgets::ACCENT),
                ))
                .clicked()
            {
                view.show_connections = !view.show_connections;
            }
        });
    });

    // ---- 实时请求列表 ----
    if view.show_connections {
        draw_connections(ui, view, store, &activity);
    }

    ui.add_space(2.0);
}

fn reset_camera(view: &mut ViewState, target: Option<(f32, f32)>) {
    let (lat, lon) = target.unwrap_or((DEFAULT_LAT, DEFAULT_LON));
    view.globe_pitch = lat;
    view.globe_yaw = lon;
    view.globe_distance = DEFAULT_DISTANCE;
}

fn paint_line(
    painter: &egui::Painter,
    position: Pos2,
    text: &str,
    font: FontId,
    color: Color32,
) -> Pos2 {
    let galley = painter.layout_no_wrap(text.to_string(), font, color);
    let height = galley.size().y + 4.0;
    painter.galley(position, galley, color);
    Pos2::new(position.x, position.y + height)
}

fn paint_legend(painter: &egui::Painter, position: Pos2, color: Color32, title: &str) -> f32 {
    painter.circle_filled(Pos2::new(position.x + 3.0, position.y + 7.0), 3.0, color);
    let galley = painter.layout_no_wrap(title.to_string(), FontId::proportional(11.0), white(191));
    let start = Pos2::new(position.x + 11.0, position.y);
    let width = galley.size().x;
    painter.galley(start, galley, white(191));
    start.x + width + 5.0
}

// ---- 连接列表（对应 connections / connectionRow） --------------------------

fn draw_connections(
    ui: &mut Ui,
    view: &mut ViewState,
    store: &Store,
    activity: &crate::state::activity::ActivityData,
) {
    if activity.connections.is_empty() {
        hint(
            ui,
            if !activity.supported {
                "当前内核不提供实时连接列表"
            } else if activity.enabled {
                "当前没有活跃请求"
            } else {
                "连接后显示实时请求"
            },
        );
        return;
    }

    ui.horizontal(|ui| {
        hint(ui, "请求目标与实际节点");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if view.focused_connection.is_some()
                && ui
                    .add(egui::Button::new(
                        RichText::new("显示全部连线")
                            .size(12.0)
                            .color(widgets::ACCENT),
                    ))
                    .clicked()
            {
                view.focused_connection = None;
            }
        });
    });
    ui.add_space(6.0);

    let listed_nodes = store.visualization_nodes();
    let index: HashMap<String, &ProxyNode> = listed_nodes
        .iter()
        .map(|node| (node.name.clone(), node))
        .collect();
    let row_height = 46.0;
    let visible = (activity.connections.len() * 65)
        .min(230)
        .max(row_height as usize) as f32;
    let shown: Vec<&CoreConnection> = activity.connections.iter().take(80).collect();
    egui::ScrollArea::vertical().max_height(visible).show_rows(
        ui,
        row_height,
        shown.len(),
        |ui, range| {
            for connection in &shown[range] {
                let focused = view.focused_connection.as_deref() == Some(connection.id.as_str());
                let frame = if focused {
                    egui::Frame::default()
                        .fill(Color32::from_rgba_unmultiplied(108, 140, 255, 26))
                        .rounding(egui::Rounding::same(6.0))
                        .inner_margin(egui::Margin::same(8.0))
                } else {
                    egui::Frame::default()
                        .fill(Color32::TRANSPARENT)
                        .rounding(egui::Rounding::same(6.0))
                        .inner_margin(egui::Margin::same(8.0))
                };
                let clicked = frame
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let response = ui.allocate_response(
                            egui::vec2(ui.available_width(), row_height - 16.0),
                            Sense::click(),
                        );
                        connection_row(ui, connection, &activity.locations, focused);
                        response.clicked()
                    })
                    .inner;
                if clicked {
                    view.focused_connection = if focused {
                        None
                    } else {
                        Some(connection.id.clone())
                    };
                    // 聚焦后镜头对准该请求的出口节点（对应 resetCamera(toward:)）。
                    let exit = connection.route_nodes(&index).last().copied();
                    if let Some(node) = exit {
                        if let Some(server) = node_server(node) {
                            if let Some(location) = activity.locations.get(&server) {
                                reset_camera(
                                    view,
                                    Some((location.latitude as f32, location.longitude as f32)),
                                );
                            }
                        }
                    }
                }
                ui.separator();
            }
        },
    );
    if activity.connections.len() > 80 {
        hint(ui, "显示流量最高的 80 个连接");
    }
}

fn connection_row(
    ui: &mut Ui,
    connection: &CoreConnection,
    locations: &HashMap<String, GeoLocation>,
    _focused: bool,
) {
    ui.horizontal(|ui| {
        let located = connection
            .metadata
            .geolocation_key()
            .and_then(|key| locations.get(&key));
        ui.label(RichText::new("●").size(9.0).color(if located.is_some() {
            ORANGE
        } else {
            widgets::MUTED
        }));
        ui.vertical(|ui| {
            ui.label(
                RichText::new(connection.metadata.display_destination())
                    .size(12.0)
                    .monospace()
                    .color(widgets::TEXT),
            );
            let path = connection.path_label();
            ui.label(
                RichText::new(if path.is_empty() {
                    "节点路径未知"
                } else {
                    &path
                })
                .size(11.0)
                .color(widgets::MUTED),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.vertical(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    let location_text = located
                        .map(|location| {
                            let estimated = connection.metadata.public_destination_ip().is_none();
                            format!(
                                "{}{}",
                                location.label(),
                                if estimated { " · DNS 估算" } else { "" }
                            )
                        })
                        .unwrap_or_else(|| "位置未解析".into());
                    ui.label(
                        RichText::new(location_text)
                            .size(11.0)
                            .color(widgets::MUTED),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "↑ {}  ↓ {}",
                            crate::logic::traffic::format_bytes(connection.upload),
                            crate::logic::traffic::format_bytes(connection.download)
                        ))
                        .size(11.0)
                        .color(widgets::MUTED),
                    );
                });
            });
        });
    });
}

// ---- 几何（与 GlobeScene 的 position / arc 一致） ---------------------------

fn unit_vector(lat: f32, lon: f32) -> (f32, f32, f32) {
    let lat = lat.to_radians();
    let lon = lon.to_radians();
    let (sin_lat, cos_lat) = lat.sin_cos();
    (cos_lat * lon.sin(), sin_lat, cos_lat * lon.cos())
}

fn rotate_toward(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    let dot = (a.0 * b.0 + a.1 * b.1 + a.2 * b.2).clamp(-1.0, 1.0);
    let angle = dot.acos();
    if angle < 1e-4 {
        return a;
    }
    let sine = angle.sin();
    let left = ((1.0 - t) * angle).sin() / sine;
    let right = (t * angle).sin() / sine;
    (
        a.0 * left + b.0 * right,
        a.1 * left + b.1 * right,
        a.2 * left + b.2 * right,
    )
}

/// 大圆弧 + 抬升：1.016 + sin(πt)·min(0.38, 0.08 + angle·0.14)，57 个采样点。
fn arc_points(from: (f32, f32), to: (f32, f32)) -> Vec<(f32, f32, f32)> {
    let a = unit_vector(from.0, from.1);
    let b = unit_vector(to.0, to.1);
    let dot = (a.0 * b.0 + a.1 * b.1 + a.2 * b.2).clamp(-1.0, 1.0);
    let angle = dot.acos();
    (0..=56)
        .map(|index| {
            let t = index as f32 / 56.0;
            let direction = rotate_toward(a, b, t);
            let lift = 1.016 + (std::f32::consts::PI * t).sin() * (0.08 + angle * 0.14).min(0.38);
            (direction.0 * lift, direction.1 * lift, direction.2 * lift)
        })
        .collect()
}

/// 正交投影到屏幕；返回 (位置, 可见度 z)。
fn project_vec(
    v: (f32, f32, f32),
    center: Pos2,
    radius: f32,
    pitch_deg: f32,
    yaw_deg: f32,
) -> (Pos2, f32) {
    let pitch = pitch_deg.to_radians();
    let yaw = yaw_deg.to_radians();
    let (sin_p, cos_p) = pitch.sin_cos();
    let (sin_y, cos_y) = yaw.sin_cos();
    // 绕 Y 轴旋转 -yaw。
    let vx = v.0 * cos_y - v.2 * sin_y;
    let vz = v.2 * cos_y + v.0 * sin_y;
    let vy = v.1;
    // 绕 X 轴旋转 pitch。
    let y_up = cos_p * vy - sin_p * vz;
    let z_view = sin_p * vy + cos_p * vz;
    (center + Vec2::new(vx, -y_up) * radius, z_view)
}

// ---- 陆地掩码与球体纹理（对应 earthTexture()） ------------------------------

const MASK_WIDTH: usize = 1024;
const MASK_HEIGHT: usize = 512;

#[derive(serde::Deserialize)]
struct Land {
    rings: Vec<Vec<[f32; 2]>>,
}

fn land() -> &'static Vec<Vec<[f32; 2]>> {
    static CACHE: OnceLock<Vec<Vec<[f32; 2]>>> = OnceLock::new();
    CACHE.get_or_init(|| match serde_json::from_str::<Land>(LAND) {
        Ok(value) => value.rings,
        Err(error) => {
            eprintln!("world land data unavailable: {error}");
            Vec::new()
        }
    })
}

fn land_mask() -> &'static [u8] {
    static CACHE: OnceLock<Vec<u8>> = OnceLock::new();
    CACHE.get_or_init(build_mask)
}

fn build_mask() -> Vec<u8> {
    let mut mask = vec![0u8; MASK_WIDTH * MASK_HEIGHT];
    for ring in land() {
        if ring.len() < 3 {
            continue;
        }
        for y in 0..MASK_HEIGHT {
            let latitude = 90.0 - (y as f32 + 0.5) * 180.0 / MASK_HEIGHT as f32;
            let mut crossings: Vec<f32> = Vec::new();
            for index in 0..ring.len() {
                let a = ring[index];
                let b = ring[(index + 1) % ring.len()];
                let (lon1, lat1) = (a[0], a[1]);
                let (lon2, lat2) = (b[0], b[1]);
                if (lat1 <= latitude && lat2 > latitude) || (lat2 <= latitude && lat1 > latitude) {
                    let ratio = (latitude - lat1) / (lat2 - lat1);
                    crossings.push(lon1 + ratio * (lon2 - lon1));
                }
            }
            crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut index = 0;
            while index + 1 < crossings.len() {
                let start = ((crossings[index] + 180.0) / 360.0 * MASK_WIDTH as f32).floor() as i32;
                let end =
                    ((crossings[index + 1] + 180.0) / 360.0 * MASK_WIDTH as f32).ceil() as i32;
                for x in start.max(0)..end.min(MASK_WIDTH as i32) {
                    mask[y * MASK_WIDTH + x as usize] = 1;
                }
                index += 2;
            }
        }
    }
    mask
}

fn globe_texture(
    ui: &Ui,
    view: &mut ViewState,
    yaw: f32,
    pitch: f32,
    size: usize,
) -> egui::TextureHandle {
    let key = (yaw, pitch, size as u32);
    if let Some(texture) = &view.globe_texture {
        if view.globe_texture_key == key {
            return texture.clone();
        }
    }
    let image = render_globe(yaw, pitch, size);
    let texture = ui
        .ctx()
        .load_texture("earth", image, egui::TextureOptions::LINEAR);
    view.globe_texture = Some(texture.clone());
    view.globe_texture_key = key;
    texture
}

fn render_globe(yaw: f32, pitch: f32, size: usize) -> egui::ColorImage {
    let mut pixels = vec![Color32::TRANSPARENT; size * size];
    let mask = land_mask();
    let center_lat = pitch.to_radians();
    let (sin_center, cos_center) = center_lat.sin_cos();
    let half = size as f32 / 2.0;
    let start_lon = yaw.to_radians();

    let is_land_at = |mx: usize, my: usize| -> bool {
        mask[(my.min(MASK_HEIGHT - 1)) * MASK_WIDTH + (mx % MASK_WIDTH)] != 0
    };

    for py in 0..size {
        let v = (py as f32 + 0.5 - half) / half;
        for px in 0..size {
            let u = (px as f32 + 0.5 - half) / half;
            let squared = u * u + v * v;
            if squared > 1.0 {
                continue;
            }
            let z = (1.0 - squared).sqrt();
            let sin_lat = (-v * cos_center + z * sin_center).clamp(-1.0, 1.0);
            let latitude = sin_lat.asin();
            let longitude = start_lon + u.atan2(z * cos_center + v * sin_center);
            let lat_deg = latitude.to_degrees();
            let mut lon_deg = longitude.to_degrees();
            while lon_deg < -180.0 {
                lon_deg += 360.0;
            }
            while lon_deg >= 180.0 {
                lon_deg -= 360.0;
            }
            let mx = (((lon_deg + 180.0) / 360.0) * MASK_WIDTH as f32) as usize % MASK_WIDTH;
            let my = (((90.0 - lat_deg) / 180.0) * MASK_HEIGHT as f32)
                .clamp(0.0, (MASK_HEIGHT - 1) as f32) as usize;
            let is_land = is_land_at(mx, my);
            // 等光照（constant）：球面颜色不随视角衰减。
            let base = if is_land { LAND_FILL } else { OCEAN };
            let mut color = base;
            // 陆地轮廓（对应 NSBezierPath stroke）。
            if is_land
                && (!is_land_at(mx.wrapping_sub(1), my)
                    || !is_land_at(mx + 1, my)
                    || !is_land_at(mx, my.wrapping_sub(1))
                    || !is_land_at(mx, my + 1))
            {
                color = LAND_STROKE;
            }
            // 经纬网格：纬线 -60..60 每 30°，经线每 30°，青色 10%。
            let on_parallel = [-60.0f32, -30.0, 0.0, 30.0, 60.0]
                .iter()
                .any(|line| (lat_deg - line).abs() < 0.4);
            let on_meridian = ((lon_deg / 30.0).round() * 30.0 - lon_deg).abs() < 0.4;
            if on_parallel || on_meridian {
                for channel in color.iter_mut() {
                    *channel = *channel * 0.90 + 25.5;
                }
            }
            let radius = squared.sqrt();
            let alpha = ((1.0 - radius) * half * 1.4).clamp(0.0, 1.0);
            pixels[py * size + px] = Color32::from_rgba_unmultiplied(
                color[0].clamp(0.0, 255.0) as u8,
                color[1].clamp(0.0, 255.0) as u8,
                color[2].clamp(0.0, 255.0) as u8,
                (alpha * 255.0) as u8,
            );
        }
    }
    egui::ColorImage {
        size: [size, size],
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, server: &str) -> ProxyNode {
        ProxyNode {
            id: id.into(),
            name: id.into(),
            source_id: "fixture".into(),
            payload: serde_json::json!({"server": server})
                .as_object()
                .unwrap()
                .clone(),
            exit_name: String::new(),
        }
    }

    fn location(lat: f64, lon: f64) -> GeoLocation {
        GeoLocation {
            latitude: lat,
            longitude: lon,
            country: "Fixture".into(),
            city: String::new(),
        }
    }

    fn trace(nodes: &[ProxyNode], selected: &str) -> ChainTrace {
        ChainTrace::resolve(selected, nodes, &[])
    }

    #[test]
    fn switching_exit_updates_markers_and_camera() {
        let nodes = vec![node("A", "a.test"), node("B", "b.test")];
        let locations = HashMap::from([
            ("a.test".into(), location(35.0, 139.0)),
            ("b.test".into(), location(1.0, 103.0)),
        ]);
        let mut view = ViewState::default();
        let first_trace = trace(&nodes, "A");
        let first = build_snapshot(&nodes, &first_trace, &[], &locations, None);
        follow_selection(&mut view, &first, &first_trace);
        assert_eq!((view.globe_pitch, view.globe_yaw), (35.0, 139.0));
        view.focused_connection = Some("old".into());
        let next_trace = trace(&nodes, "B");
        let next = build_snapshot(&nodes, &next_trace, &[], &locations, None);
        follow_selection(&mut view, &next, &next_trace);
        assert_eq!(next.selected_node_id.as_deref(), Some("B"));
        assert_eq!(
            next.markers
                .iter()
                .filter(|m| m.active)
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["node:b.test"]
        );
        assert_eq!((view.globe_pitch, view.globe_yaw), (1.0, 103.0));
        assert_eq!(next.mapped_nodes, 2);
        assert!(view.focused_connection.is_none());
    }

    #[test]
    fn multihop_follows_generated_exit_not_first_marker() {
        let entry = node("entry", "a-entry.test");
        let mut exit = node("generated-exit", "z-exit.test");
        exit.payload
            .insert("dialer-proxy".into(), serde_json::json!(entry.name));
        let nodes = vec![entry, exit];
        let trace = trace(&nodes, "generated-exit");
        let locations = HashMap::from([
            ("a-entry.test".into(), location(35.0, 139.0)),
            ("z-exit.test".into(), location(1.0, 103.0)),
        ]);
        let snapshot = build_snapshot(&nodes, &trace, &[], &locations, None);
        assert!(snapshot.markers.first().unwrap().active);
        assert_eq!(snapshot.selected_location, Some((1.0, 103.0)));
        assert_eq!(snapshot.selected_node_id.as_deref(), Some("generated-exit"));
        assert_eq!(snapshot.routes[0].locations, [(35.0, 139.0), (1.0, 103.0)]);
        let mut view = ViewState::default();
        follow_selection(&mut view, &snapshot, &trace);
        assert_eq!((view.globe_pitch, view.globe_yaw), (1.0, 103.0));
    }

    #[test]
    fn pending_exit_does_not_fall_back_and_late_location_follows() {
        let nodes = vec![node("A", "a.test"), node("B", "b.test")];
        let mut locations = HashMap::from([("a.test".into(), location(35.0, 139.0))]);
        let mut view = ViewState::default();
        let a_trace = trace(&nodes, "A");
        follow_selection(
            &mut view,
            &build_snapshot(&nodes, &a_trace, &[], &locations, None),
            &a_trace,
        );
        view.globe_yaw = -10.0;
        let b_trace = trace(&nodes, "B");
        let pending = build_snapshot(&nodes, &b_trace, &[], &locations, None);
        assert_eq!(pending.selected_node_id.as_deref(), Some("B"));
        assert!(pending.selected_location.is_none());
        assert!(pending.markers.iter().all(|m| !m.active));
        follow_selection(&mut view, &pending, &b_trace);
        assert_eq!(view.globe_yaw, -10.0);
        locations.insert("b.test".into(), location(1.0, 103.0));
        follow_selection(
            &mut view,
            &build_snapshot(&nodes, &b_trace, &[], &locations, None),
            &b_trace,
        );
        assert_eq!((view.globe_pitch, view.globe_yaw), (1.0, 103.0));
        // An older request completing after B cannot steal focus back to A.
        locations.insert("a.test".into(), location(40.0, 140.0));
        follow_selection(
            &mut view,
            &build_snapshot(&nodes, &b_trace, &[], &locations, None),
            &b_trace,
        );
        assert_eq!((view.globe_pitch, view.globe_yaw), (1.0, 103.0));
    }

    #[test]
    fn polling_same_exit_preserves_manual_rotation_and_zoom() {
        let nodes = vec![node("A", "a.test")];
        let trace = trace(&nodes, "A");
        let locations = HashMap::from([
            ("a.test".into(), location(35.0, 139.0)),
            ("1.1.1.1".into(), location(-27.0, 153.0)),
        ]);
        let snapshot = build_snapshot(&nodes, &trace, &[], &locations, None);
        let mut view = ViewState::default();
        follow_selection(&mut view, &snapshot, &trace);
        view.globe_pitch = -20.0;
        view.globe_yaw = -30.0;
        view.globe_distance = 6.0;
        let connection = CoreConnection {
            id: "request".into(),
            chains: vec!["A".into()],
            metadata: crate::logic::model::ConnectionMetadata {
                destination_ip: Some("1.1.1.1".into()),
                ..Default::default()
            },
            upload: 0,
            download: 0,
            rule: None,
        };
        let polled = build_snapshot(&nodes, &trace, &[connection], &locations, Some("request"));
        assert_eq!(polled.mapped_connections, 1);
        follow_selection(&mut view, &polled, &trace);
        assert_eq!(
            (view.globe_pitch, view.globe_yaw, view.globe_distance),
            (-20.0, -30.0, 6.0)
        );
    }

    #[test]
    fn switching_ids_on_shared_server_still_follows() {
        let nodes = vec![node("A", "shared.test"), node("B", "shared.test")];
        let locations = HashMap::from([("shared.test".into(), location(35.0, 139.0))]);
        let mut view = ViewState::default();
        let a_trace = trace(&nodes, "A");
        follow_selection(
            &mut view,
            &build_snapshot(&nodes, &a_trace, &[], &locations, None),
            &a_trace,
        );
        view.globe_yaw = -50.0;
        let b_trace = trace(&nodes, "B");
        let next = build_snapshot(&nodes, &b_trace, &[], &locations, None);
        assert_eq!(next.markers.len(), 1);
        follow_selection(&mut view, &next, &b_trace);
        assert_eq!(view.globe_yaw, 139.0);
        assert_eq!(view.globe_selected_node.as_deref(), Some("B"));
    }

    #[test]
    fn unknown_incomplete_and_invalid_exits_never_offer_focus() {
        let nodes = vec![node("A", "a.test")];
        let valid_trace = trace(&nodes, "A");
        let locations = HashMap::from([("a.test".into(), location(91.0, 139.0))]);
        let invalid = build_snapshot(&nodes, &valid_trace, &[], &locations, None);
        assert!(invalid.selected_location.is_none());
        let incomplete = ChainTrace {
            hops: valid_trace.hops.clone(),
            complete: false,
        };
        let snapshot = build_snapshot(&nodes, &incomplete, &[], &locations, None);
        assert!(snapshot.selected_node_id.is_none());
        assert!(snapshot.selected_location.is_none());
        let unknown = build_snapshot(&[], &valid_trace, &[], &locations, None);
        assert!(unknown.selected_node_id.is_none());
        assert!(unknown.selected_location.is_none());
    }

    #[test]
    fn changed_coordinates_for_same_exit_refocus_camera() {
        let nodes = vec![node("A", "a.test")];
        let trace = trace(&nodes, "A");
        let mut locations = HashMap::from([("a.test".into(), location(35.0, 139.0))]);
        let mut view = ViewState::default();
        follow_selection(
            &mut view,
            &build_snapshot(&nodes, &trace, &[], &locations, None),
            &trace,
        );
        view.globe_yaw = -40.0;
        locations.insert("a.test".into(), location(1.0, 103.0));
        follow_selection(
            &mut view,
            &build_snapshot(&nodes, &trace, &[], &locations, None),
            &trace,
        );
        assert_eq!((view.globe_pitch, view.globe_yaw), (1.0, 103.0));
    }

    #[test]
    fn land_data_is_readable() {
        let rings = land();
        assert!(rings.len() > 100);
        assert!(rings.iter().map(|ring| ring.len()).sum::<usize>() > 2000);
    }

    #[test]
    fn arc_lifts_and_returns() {
        let points = arc_points((0.0, 0.0), (30.0, 120.0));
        assert_eq!(points.len(), 57);
        let start = points[0];
        let end = points[56];
        let length = |p: (f32, f32, f32)| (p.0 * p.0 + p.1 * p.1 + p.2 * p.2).sqrt();
        assert!((length(start) - 1.016).abs() < 0.01);
        assert!((length(end) - 1.016).abs() < 0.01);
    }
}
