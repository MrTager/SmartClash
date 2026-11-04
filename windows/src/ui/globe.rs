use super::widgets::{self, hint, ACCENT, GOOD, MUTED};
use super::ViewState;
use crate::state::Store;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::HashMap;
use std::sync::OnceLock;

const LAND: &str = include_str!("../../resources/world-land.json");

struct Region {
    code: &'static str,
    label: &'static str,
    lat: f32,
    lon: f32,
    keywords: &'static [&'static str],
}

const REGIONS: &[Region] = &[
    Region { code: "HK", label: "香港", lat: 22.3, lon: 114.2, keywords: &["香港", "深港", "沙田", "Hong Kong", "HongKong"] },
    Region { code: "MO", label: "澳门", lat: 22.2, lon: 113.5, keywords: &["澳门", "Macau", "Macao"] },
    Region { code: "TW", label: "台湾", lat: 23.7, lon: 121.0, keywords: &["台湾", "台北", "新北", "彰化", "Taiwan", "Taipei"] },
    Region { code: "JP", label: "日本", lat: 36.0, lon: 138.0, keywords: &["日本", "东京", "大阪", "名古屋", "埼玉", "Japan", "Tokyo", "Osaka"] },
    Region { code: "KR", label: "韩国", lat: 36.5, lon: 127.8, keywords: &["韩国", "首尔", "Korea", "Seoul"] },
    Region { code: "SG", label: "新加坡", lat: 1.35, lon: 103.8, keywords: &["新加坡", "狮城", "Singapore"] },
    Region { code: "MY", label: "马来西亚", lat: 3.1, lon: 101.7, keywords: &["马来西亚", "吉隆坡", "Malaysia", "Kuala Lumpur"] },
    Region { code: "TH", label: "泰国", lat: 13.75, lon: 100.5, keywords: &["泰国", "曼谷", "Thailand", "Bangkok"] },
    Region { code: "VN", label: "越南", lat: 21.0, lon: 105.8, keywords: &["越南", "河内", "胡志明", "Vietnam"] },
    Region { code: "PH", label: "菲律宾", lat: 14.6, lon: 121.0, keywords: &["菲律宾", "马尼拉", "Philippines", "Manila"] },
    Region { code: "ID", label: "印度尼西亚", lat: -6.2, lon: 106.8, keywords: &["印尼", "印度尼西亚", "雅加达", "Indonesia", "Jakarta"] },
    Region { code: "IN", label: "印度", lat: 20.0, lon: 77.0, keywords: &["印度", "孟买", "India", "Mumbai"] },
    Region { code: "PK", label: "巴基斯坦", lat: 30.0, lon: 70.0, keywords: &["巴基斯坦", "Pakistan"] },
    Region { code: "BD", label: "孟加拉", lat: 23.7, lon: 90.4, keywords: &["孟加拉", "Bangladesh"] },
    Region { code: "LK", label: "斯里兰卡", lat: 7.0, lon: 80.7, keywords: &["斯里兰卡", "Sri Lanka"] },
    Region { code: "NP", label: "尼泊尔", lat: 27.7, lon: 85.3, keywords: &["尼泊尔", "Nepal"] },
    Region { code: "KH", label: "柬埔寨", lat: 11.5, lon: 104.9, keywords: &["柬埔寨", "Cambodia"] },
    Region { code: "MM", label: "缅甸", lat: 16.8, lon: 96.2, keywords: &["缅甸", "Myanmar"] },
    Region { code: "MN", label: "蒙古", lat: 47.9, lon: 106.9, keywords: &["蒙古", "Mongolia"] },
    Region { code: "KZ", label: "哈萨克斯坦", lat: 48.0, lon: 67.0, keywords: &["哈萨克", "Kazakhstan"] },
    Region { code: "CN", label: "中国", lat: 35.0, lon: 105.0, keywords: &["中国", "内地", "回国", "China"] },
    Region { code: "AE", label: "阿联酋", lat: 24.4, lon: 54.4, keywords: &["阿联酋", "迪拜", "UAE", "Dubai", "Emirates"] },
    Region { code: "SA", label: "沙特", lat: 24.7, lon: 46.7, keywords: &["沙特", "Saudi"] },
    Region { code: "QA", label: "卡塔尔", lat: 25.3, lon: 51.5, keywords: &["卡塔尔", "Qatar"] },
    Region { code: "IL", label: "以色列", lat: 31.8, lon: 35.0, keywords: &["以色列", "Israel"] },
    Region { code: "TR", label: "土耳其", lat: 39.0, lon: 35.0, keywords: &["土耳其", "伊斯坦布尔", "Turkey", "Istanbul"] },
    Region { code: "RU", label: "俄罗斯", lat: 55.7, lon: 37.6, keywords: &["俄罗斯", "莫斯科", "圣彼得堡", "Russia", "Moscow"] },
    Region { code: "UA", label: "乌克兰", lat: 50.4, lon: 30.5, keywords: &["乌克兰", "Ukraine"] },
    Region { code: "DE", label: "德国", lat: 50.1, lon: 8.7, keywords: &["德国", "法兰克福", "柏林", "慕尼黑", "Germany", "Frankfurt", "Berlin"] },
    Region { code: "GB", label: "英国", lat: 51.5, lon: -0.1, keywords: &["英国", "伦敦", "United Kingdom", "Britain", "London"] },
    Region { code: "FR", label: "法国", lat: 48.9, lon: 2.35, keywords: &["法国", "巴黎", "France", "Paris"] },
    Region { code: "NL", label: "荷兰", lat: 52.4, lon: 4.9, keywords: &["荷兰", "阿姆斯特丹", "Netherlands", "Amsterdam"] },
    Region { code: "BE", label: "比利时", lat: 50.8, lon: 4.35, keywords: &["比利时", "Belgium"] },
    Region { code: "CH", label: "瑞士", lat: 46.9, lon: 7.4, keywords: &["瑞士", "Switzerland", "Zurich"] },
    Region { code: "AT", label: "奥地利", lat: 48.2, lon: 16.4, keywords: &["奥地利", "Austria", "Vienna"] },
    Region { code: "IT", label: "意大利", lat: 41.9, lon: 12.5, keywords: &["意大利", "米兰", "Italy", "Milan"] },
    Region { code: "ES", label: "西班牙", lat: 40.4, lon: -3.7, keywords: &["西班牙", "Spain", "Madrid"] },
    Region { code: "SE", label: "瑞典", lat: 59.3, lon: 18.1, keywords: &["瑞典", "Sweden", "Stockholm"] },
    Region { code: "NO", label: "挪威", lat: 59.9, lon: 10.7, keywords: &["挪威", "Norway", "Oslo"] },
    Region { code: "DK", label: "丹麦", lat: 55.7, lon: 12.6, keywords: &["丹麦", "Denmark"] },
    Region { code: "FI", label: "芬兰", lat: 60.2, lon: 24.9, keywords: &["芬兰", "Finland", "Helsinki"] },
    Region { code: "PL", label: "波兰", lat: 52.2, lon: 21.0, keywords: &["波兰", "Poland", "Warsaw"] },
    Region { code: "IE", label: "爱尔兰", lat: 53.3, lon: -6.3, keywords: &["爱尔兰", "Ireland", "Dublin"] },
    Region { code: "CZ", label: "捷克", lat: 50.1, lon: 14.4, keywords: &["捷克", "Czech", "Prague"] },
    Region { code: "PT", label: "葡萄牙", lat: 38.7, lon: -9.1, keywords: &["葡萄牙", "Portugal", "Lisbon"] },
    Region { code: "GR", label: "希腊", lat: 38.0, lon: 23.7, keywords: &["希腊", "Greece", "Athens"] },
    Region { code: "HU", label: "匈牙利", lat: 47.5, lon: 19.0, keywords: &["匈牙利", "Hungary", "Budapest"] },
    Region { code: "RO", label: "罗马尼亚", lat: 44.4, lon: 26.1, keywords: &["罗马尼亚", "Romania"] },
    Region { code: "US", label: "美国", lat: 38.0, lon: -97.0, keywords: &["美国", "洛杉矶", "圣何塞", "西雅图", "纽约", "达拉斯", "芝加哥", "凤凰城", "迈阿密", "硅谷", "United States", "America", "Los Angeles", "San Jose", "Seattle", "New York", "Dallas"] },
    Region { code: "CA", label: "加拿大", lat: 56.0, lon: -106.0, keywords: &["加拿大", "多伦多", "温哥华", "蒙特利尔", "Canada", "Toronto", "Vancouver"] },
    Region { code: "MX", label: "墨西哥", lat: 23.6, lon: -102.0, keywords: &["墨西哥", "Mexico"] },
    Region { code: "BR", label: "巴西", lat: -15.8, lon: -47.9, keywords: &["巴西", "圣保罗", "Brazil", "Sao Paulo"] },
    Region { code: "AR", label: "阿根廷", lat: -34.6, lon: -58.4, keywords: &["阿根廷", "Argentina"] },
    Region { code: "CL", label: "智利", lat: -33.4, lon: -70.7, keywords: &["智利", "Chile"] },
    Region { code: "CO", label: "哥伦比亚", lat: 4.7, lon: -74.1, keywords: &["哥伦比亚", "Colombia"] },
    Region { code: "PE", label: "秘鲁", lat: -12.0, lon: -77.0, keywords: &["秘鲁", "Peru"] },
    Region { code: "ZA", label: "南非", lat: -26.2, lon: 28.0, keywords: &["南非", "South Africa"] },
    Region { code: "EG", label: "埃及", lat: 30.0, lon: 31.2, keywords: &["埃及", "Egypt"] },
    Region { code: "NG", label: "尼日利亚", lat: 9.1, lon: 7.4, keywords: &["尼日利亚", "Nigeria"] },
    Region { code: "KE", label: "肯尼亚", lat: -1.3, lon: 36.8, keywords: &["肯尼亚", "Kenya"] },
    Region { code: "MA", label: "摩洛哥", lat: 33.6, lon: -7.6, keywords: &["摩洛哥", "Morocco"] },
    Region { code: "AU", label: "澳大利亚", lat: -33.9, lon: 151.2, keywords: &["澳大利亚", "澳洲", "悉尼", "墨尔本", "Australia", "Sydney", "Melbourne"] },
    Region { code: "NZ", label: "新西兰", lat: -41.3, lon: 174.8, keywords: &["新西兰", "New Zealand"] },
];

fn has_token(text: &str, token: &str) -> bool {
    let bytes = text.as_bytes();
    let mut start = 0;
    while let Some(offset) = text[start..].find(token) {
        let position = start + offset;
        let before_ok = position == 0 || !bytes[position - 1].is_ascii_alphanumeric();
        let end = position + token.len();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = position + token.len();
    }
    false
}

fn flag(code: &str) -> String {
    let mut out = String::new();
    for letter in code.chars() {
        if let Some(value) = (letter as u32).checked_sub('A' as u32) {
            if let Some(ch) = char::from_u32(0x1F1E6 + value) {
                out.push(ch);
            }
        }
    }
    out
}

fn region_index(name: &str) -> Option<usize> {
    let lower = name.to_lowercase();
    for (index, region) in REGIONS.iter().enumerate() {
        if name.contains(&flag(region.code)) || has_token(name, region.code) {
            return Some(index);
        }
        for keyword in region.keywords {
            if keyword.is_ascii() {
                if has_token(&lower, &keyword.to_lowercase()) {
                    return Some(index);
                }
            } else if name.contains(keyword) {
                return Some(index);
            }
        }
    }
    None
}

fn is_info_node(name: &str) -> bool {
    const MARKERS: [&str; 14] = [
        "剩余", "流量", "到期", "官网", "公告", "套餐", "续费", "客服", "邀请", "注册", "网址",
        "订阅", "重置", "过期",
    ];
    MARKERS.iter().any(|marker| name.contains(marker))
}

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

pub fn draw(store: &Store, ui: &mut Ui, view: &mut ViewState) {
    let mut counts: HashMap<usize, usize> = HashMap::new();
    let mut recognised = 0usize;
    let mut total = 0usize;
    for node in &store.nodes {
        if is_info_node(&node.name) {
            continue;
        }
        total += 1;
        if let Some(index) = region_index(&node.name) {
            *counts.entry(index).or_insert(0) += 1;
            recognised += 1;
        }
    }

    let active_region = store
        .nodes
        .iter()
        .find(|node| node.name == store.selected)
        .and_then(|node| region_index(&node.name));

    let mut entries: Vec<(usize, usize)> = counts.iter().map(|(index, count)| (*index, *count)).collect();
    entries.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| REGIONS[a.0].label.cmp(REGIONS[b.0].label))
    });
    let hub = active_region.or_else(|| entries.first().map(|entry| entry.0));

    widgets::card(ui, Some("全球节点分布"), |ui| {
        ui.horizontal(|ui| {
            hint(ui, "按节点名称推断地区 · 拖动地球旋转 · 双击复位");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut view.globe_auto, "自动旋转");
            });
        });
        ui.add_space(4.0);

        let width = ui.available_width().max(280.0);
        let height = (width * 0.5).clamp(240.0, 340.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());

        if response.dragged() {
            let delta = response.drag_delta();
            view.globe_yaw -= delta.x * 0.4;
            view.globe_pitch = (view.globe_pitch + delta.y * 0.3).clamp(-72.0, 72.0);
        }
        if response.double_clicked() {
            view.globe_yaw = 105.0;
            view.globe_pitch = 20.0;
        }
        let hovering = response.hovered();
        if view.globe_auto && !response.dragged() && !hovering {
            let delta = ui.input(|input| input.stable_dt).min(0.2);
            view.globe_yaw += delta * 3.0;
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        } else if !counts.is_empty() {
            // 只重绘流动点，不重新生成地球纹理。
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        }
        if view.globe_yaw > 180.0 {
            view.globe_yaw -= 360.0;
        } else if view.globe_yaw < -180.0 {
            view.globe_yaw += 360.0;
        }

        let painter = ui.painter_at(rect);
        let center = rect.center();
        let radius = (rect.height() * 0.44).min(rect.width() * 0.24);
        let yaw = view.globe_yaw;
        let pitch = view.globe_pitch;

        painter.circle_filled(center, radius + 12.0, Color32::from_rgba_unmultiplied(84, 152, 255, 8));
        painter.circle_filled(center, radius + 6.0, Color32::from_rgba_unmultiplied(84, 152, 255, 12));

        let pixels = (radius * 2.8).clamp(256.0, 512.0) as usize;
        let texture = globe_texture(ui, view, yaw, pitch, pixels);
        painter.image(
            texture.id(),
            Rect::from_center_size(center, Vec2::splat(radius * 2.0)),
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        painter.circle_stroke(center, radius, Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(84, 152, 255, 90)));

        if let Some(hub) = hub {
            let hub_region = &REGIONS[hub];
            let now = ui.input(|input| input.time) as f32;
            let top = entries.first().map(|entry| entry.1).unwrap_or(1).max(1);
            for (order, (index, count)) in entries.iter().take(14).enumerate() {
                if *index == hub {
                    continue;
                }
                let region = &REGIONS[*index];
                let alpha = 46 + ((*count as f32 / top as f32) * 70.0) as u8;
                let color = Color32::from_rgba_unmultiplied(89, 205, 224, alpha);
                draw_arc(
                    &painter,
                    (hub_region.lat, hub_region.lon),
                    (region.lat, region.lon),
                    center,
                    radius,
                    yaw,
                    pitch,
                    color,
                    0.05,
                );
                if order < 7 {
                    let progress = (now * 0.16 + order as f32 * 0.17).fract();
                    if let Some((position, z)) = arc_screen(
                        (hub_region.lat, hub_region.lon),
                        (region.lat, region.lon),
                        progress,
                        center,
                        radius,
                        yaw,
                        pitch,
                        0.08,
                    ) {
                        if z > 0.02 {
                            painter.circle_filled(
                                position,
                                2.1,
                                Color32::from_rgba_unmultiplied(150, 226, 240, 210),
                            );
                        }
                    }
                }
            }
        }

        for (index, count) in &counts {
            let region = &REGIONS[*index];
            let (mut position, z) = project(region.lat, region.lon, center, radius, pitch, yaw);
            if z <= 0.04 {
                continue;
            }
            let size = 2.6 + (*count as f32).sqrt() * 1.15;
            let active = active_region == Some(*index);
            let color = if active { GOOD } else { ACCENT };

            painter.circle_filled(position, size * 2.4, Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 20));
            painter.circle_filled(position, size, color);
            painter.circle_stroke(position, size, Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(10, 16, 28, 200)));
            if active {
                let pulse = 1.0 + ((ui.input(|input| input.time) * 2.0).sin() as f32 + 1.0) * 1.4;
                painter.circle_stroke(position, size + pulse + 3.0, Stroke::new(1.6_f32, Color32::from_rgba_unmultiplied(88, 199, 132, 150)));
            }
            position.y += 0.0;
        }

        let mut labels: Vec<(usize, Pos2, f32, bool)> = counts
            .iter()
            .filter_map(|(index, count)| {
                let region = &REGIONS[*index];
                let (position, z) = project(region.lat, region.lon, center, radius, pitch, yaw);
                if z > 0.08 {
                    Some((*index, position, *count as f32, active_region == Some(*index)))
                } else {
                    None
                }
            })
            .collect();
        labels.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| REGIONS[a.0].label.cmp(REGIONS[b.0].label))
        });

        let mut placed: Vec<Rect> = Vec::new();
        for (index, position, count, active) in labels {
            let region = &REGIONS[index];
            let color = if active {
                Color32::from_rgb(88, 199, 132)
            } else {
                Color32::from_rgb(206, 216, 232)
            };
            let text = format!("{} {}", region.label, count as usize);
            let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), color);
            let size = galley.size();
            let offset = 5.0;
            let candidates = [
                Pos2::new(position.x + offset, position.y - size.y - offset),
                Pos2::new(position.x + offset, position.y + offset),
                Pos2::new(position.x - size.x - offset, position.y - size.y - offset),
                Pos2::new(position.x - size.x - offset, position.y + offset),
            ];
            for start in candidates {
                let box_rect = Rect::from_min_size(start, size);
                if box_rect.min.x < rect.min.x || box_rect.max.x > rect.max.x || box_rect.min.y < rect.min.y || box_rect.max.y > rect.max.y {
                    continue;
                }
                if placed.iter().any(|existing| existing.intersects(box_rect)) {
                    continue;
                }
                placed.push(box_rect);
                painter.galley(start, galley, color);
                break;
            }
        }

        if let Some(hover) = response.hover_pos() {
            let nearest = counts
                .iter()
                .filter_map(|(index, count)| {
                    let region = &REGIONS[*index];
                    let (position, z) = project(region.lat, region.lon, center, radius, pitch, yaw);
                    if z <= 0.04 {
                        return None;
                    }
                    let distance = position.distance(hover);
                    (distance <= 14.0).then_some((distance, *index, *count))
                })
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((_, index, count)) = nearest {
                response.clone().on_hover_text(format!("{} · {} 个节点", REGIONS[index].label, count));
            }
        }

        if total == 0 {
            hint(ui, "还没有节点；导入订阅后这里会显示各出口的地区分布。");
        } else {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                for (index, count) in &entries {
                    let region = &REGIONS[*index];
                    let color = if active_region == Some(*index) { GOOD } else { ACCENT };
                    ui.label(RichText::new("●").size(13.0).color(color));
                    ui.label(RichText::new(format!("{} {}", region.label, count)).size(12.0));
                    ui.add_space(6.0);
                }
            });
            let unknown = total.saturating_sub(recognised);
            if unknown > 0 {
                hint(
                    ui,
                    &format!("{recognised}/{total} 个节点可识别地区，{unknown} 个名称未包含可判断的国家或城市。"),
                );
            } else {
                hint(ui, &format!("{recognised} 个节点全部已定位。"));
            }
        }
        ui.add_space(2.0);
        hint(
            ui,
            "连线表示当前出口（未连接时为节点最集中的地区）与其他节点地区的对应关系，不代表实时请求路径；坐标取国家或代表城市中心，不代表真实机房位置。",
        );
    });
}

fn unit_vector(lat: f32, lon: f32) -> (f32, f32, f32) {
    let lat = lat.to_radians();
    let lon = lon.to_radians();
    let (sin_lat, cos_lat) = lat.sin_cos();
    (cos_lat * lon.cos(), sin_lat, cos_lat * lon.sin())
}

fn arc_screen(
    from: (f32, f32),
    to: (f32, f32),
    t: f32,
    center: Pos2,
    radius: f32,
    yaw: f32,
    pitch: f32,
    altitude: f32,
) -> Option<(Pos2, f32)> {
    let (ax, ay, az) = unit_vector(from.0, from.1);
    let (bx, by, bz) = unit_vector(to.0, to.1);
    let dot = (ax * bx + ay * by + az * bz).clamp(-1.0, 1.0);
    let angle = dot.acos();
    let (mut x, mut y, mut z) = if angle < 1e-4 {
        (ax, ay, az)
    } else {
        let sine = angle.sin();
        let left = ((1.0 - t) * angle).sin() / sine;
        let right = (t * angle).sin() / sine;
        (ax * left + bx * right, ay * left + by * right, az * left + bz * right)
    };
    let length = (x * x + y * y + z * z).sqrt();
    if length < 1e-5 {
        return None;
    }
    x /= length;
    y /= length;
    z /= length;
    let lift = 1.0 + altitude * (std::f32::consts::PI * t).sin();
    let lat = y.asin().to_degrees();
    let lon = z.atan2(x).to_degrees();
    let (position, visible) = project(lat, lon, center, radius * lift, pitch, yaw);
    Some((position, visible))
}

fn draw_arc(
    painter: &egui::Painter,
    from: (f32, f32),
    to: (f32, f32),
    center: Pos2,
    radius: f32,
    yaw: f32,
    pitch: f32,
    color: Color32,
    altitude: f32,
) {
    let mut previous: Option<(Pos2, f32)> = None;
    const STEPS: usize = 48;
    for step in 0..=STEPS {
        let t = step as f32 / STEPS as f32;
        if let Some((position, visible)) = arc_screen(from, to, t, center, radius, yaw, pitch, altitude) {
            if let Some((last, last_visible)) = previous {
                if visible > 0.02 && last_visible > 0.02 {
                    painter.line_segment([last, position], Stroke::new(1.1_f32, color));
                }
            }
            previous = Some((position, visible));
        }
    }
}

fn project(lat: f32, lon: f32, center: Pos2, radius: f32, center_lat: f32, center_lon: f32) -> (Pos2, f32) {
    let lat = lat.to_radians();
    let delta = (lon - center_lon).to_radians();
    let center_lat = center_lat.to_radians();
    let x = lat.cos() * delta.sin();
    let y = center_lat.cos() * lat.sin() - center_lat.sin() * lat.cos() * delta.cos();
    let z = center_lat.sin() * lat.sin() + center_lat.cos() * lat.cos() * delta.cos();
    (center + Vec2::new(x, -y) * radius, z)
}

const MASK_WIDTH: usize = 1024;
const MASK_HEIGHT: usize = 512;

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
                let end = ((crossings[index + 1] + 180.0) / 360.0 * MASK_WIDTH as f32).ceil() as i32;
                for x in start.max(0)..end.min(MASK_WIDTH as i32) {
                    mask[y * MASK_WIDTH + x as usize] = 1;
                }
                index += 2;
            }
        }
    }
    mask
}

fn globe_texture(ui: &Ui, view: &mut ViewState, yaw: f32, pitch: f32, size: usize) -> egui::TextureHandle {
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
            let my = (((90.0 - lat_deg) / 180.0) * MASK_HEIGHT as f32).clamp(0.0, (MASK_HEIGHT - 1) as f32) as usize;
            let is_land = mask[my * MASK_WIDTH + mx] != 0;
            let light = 0.62 + 0.38 * z;
            let (red, green, blue) = if is_land {
                (40.0 * light, 100.0 * light, 108.0 * light)
            } else {
                (10.0 * light, 30.0 * light, 52.0 * light)
            };
            let mut color = [red, green, blue];
            let lat_remainder = lat_deg.rem_euclid(30.0);
            let lon_remainder = lon_deg.rem_euclid(30.0);
            let on_parallel = lat_remainder.min(30.0 - lat_remainder) < 0.5;
            let on_meridian = lon_remainder.min(30.0 - lon_remainder) < 0.5;
            if on_parallel || on_meridian {
                for (channel, target) in color.iter_mut().zip([104.0, 142.0, 178.0]) {
                    *channel += (target - *channel) * 0.16;
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

    #[test]
    fn recognises_common_node_names() {
        assert_eq!(region_index("🇭🇰 香港 01").map(|i| REGIONS[i].code), Some("HK"));
        assert_eq!(region_index("JP-Tokyo-01").map(|i| REGIONS[i].code), Some("JP"));
        assert_eq!(region_index("🇸🇬 SG 02").map(|i| REGIONS[i].code), Some("SG"));
        assert_eq!(region_index("美国 洛杉矶 03").map(|i| REGIONS[i].code), Some("US"));
        assert_eq!(region_index("德国 法兰克福").map(|i| REGIONS[i].code), Some("DE"));
        assert_eq!(region_index("剩余流量：100GB").map(|i| REGIONS[i].code), None);
        assert!(is_info_node("套餐到期时间"));
    }

    #[test]
    fn land_data_is_readable() {
        let rings = land();
        assert!(rings.len() > 100);
        assert!(rings.iter().map(|ring| ring.len()).sum::<usize>() > 2000);
    }
}
