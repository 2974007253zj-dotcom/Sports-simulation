//! Shared responsive and Android input helpers.

use crate::platform::InputKind;
use crate::ui::theme;
use eframe::egui;
use std::{
    hash::Hash,
    ops::{Range, RangeInclusive},
};

pub const COMPACT_BREAKPOINT: f32 = 620.0;
pub const TOUCH_HEIGHT: f32 = 44.0;

pub fn is_compact(width: f32) -> bool {
    width <= COMPACT_BREAKPOINT
}

pub fn compact_ui(ui: &egui::Ui) -> bool {
    is_compact(ui.available_width())
}

pub fn row<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    if compact_ui(ui) {
        ui.horizontal_wrapped(add_contents)
    } else {
        ui.horizontal(add_contents)
    }
}

/// 「标签 + 控件」的一行。
///
/// 桌面端同排（标签在左、控件在右）；手机端**标签独占一行**、控件另起一行。
/// 直接用 `horizontal_wrapped` 时，固定宽度的输入框会让整行溢出屏幕右侧，
/// 而 egui 不会把单个元素拆开，所以必须显式分行。
pub fn labeled<R>(
    ui: &mut egui::Ui,
    label: &str,
    add_control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    if compact_ui(ui) {
        ui.label(label);
        ui.indent("labeled_ctl", |ui| add_control(ui)).inner
    } else {
        ui.horizontal(|ui| {
            ui.label(label);
            add_control(ui)
        })
        .inner
    }
}

#[cfg(any(target_os = "android", test))]
pub fn parse_i64_in_range(input: &str, range: RangeInclusive<i64>) -> Result<i64, String> {
    let value = input
        .trim()
        .parse::<i64>()
        .map_err(|_| "请输入整数".to_owned())?;
    if range.contains(&value) {
        Ok(value)
    } else {
        Err(format!("请输入 {} 到 {}", range.start(), range.end()))
    }
}

#[cfg(any(target_os = "android", test))]
pub fn parse_f32_in_range(input: &str, range: RangeInclusive<f32>) -> Result<f32, String> {
    let value = input
        .trim()
        .parse::<f32>()
        .map_err(|_| "请输入数字".to_owned())?;
    if value.is_finite() && range.contains(&value) {
        Ok(value)
    } else {
        Err(format!("请输入 {} 到 {}", range.start(), range.end()))
    }
}

#[cfg(target_os = "android")]
fn parse_f64_in_range(input: &str, range: RangeInclusive<f64>) -> Result<f64, String> {
    let value = input
        .trim()
        .parse::<f64>()
        .map_err(|_| "请输入数字".to_owned())?;
    if value.is_finite() && range.contains(&value) {
        Ok(value)
    } else {
        Err(format!("请输入 {} 到 {}", range.start(), range.end()))
    }
}

/// 单个标签：hover 淡底 + 选中底部 2px 高亮条（不用背景块）。
fn tab_item(ui: &mut egui::Ui, size: egui::Vec2, label: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, egui::Rounding::same(theme::RADIUS_CONTROL), theme::WHITE);
        } else if response.hovered() {
            painter.rect_filled(
                rect,
                egui::Rounding::same(theme::RADIUS_CONTROL),
                theme::PRIMARY_TINT,
            );
        }

        let color = if selected {
            theme::PRIMARY
        } else {
            theme::TEXT_SECONDARY
        };
        let galley = painter.layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(theme::FONT_BODY),
            color,
        );
        let text_pos = rect.center() - galley.size() / 2.0;
        painter.galley(text_pos, galley, color);

        if selected {
            let bar_width = (size.x - theme::GAP_SM * 2.0).max(16.0);
            let bar = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, rect.bottom() - 1.0),
                egui::vec2(bar_width, 2.0),
            );
            painter.rect_filled(bar, egui::Rounding::same(1.0), theme::PRIMARY);
        }
    }
    response
}

/// 把 `total` 项尽量均分成 `rows` 行，返回每行的下标区间（前面的行多 1 项）。
fn balanced_chunks(total: usize, rows: usize) -> Vec<Range<usize>> {
    (0..rows)
        .map(|row| {
            let base = total / rows;
            let extra = total % rows;
            let start = row * base + row.min(extra);
            let len = base + usize::from(row < extra);
            start..start + len
        })
        .collect()
}

/// 标签页索引常量。跳转到某个页面时必须用这里的常量，不要写死数字：
/// 原先「检查更新」用 `self.tab = 8`，而 8 是「配置档位」，关于页是 9，
/// 导致点检查更新会跳到配置档位。
pub const TAB_RUN: usize = 0;
pub const TAB_AI: usize = 1;
pub const TAB_RECORDS: usize = 2;
pub const TAB_DATA: usize = 3;
pub const TAB_USER: usize = 4;
pub const TAB_DEVICE: usize = 5;
pub const TAB_LOG: usize = 6;
pub const TAB_OSM: usize = 7;
pub const TAB_PROFILES: usize = 8;
pub const TAB_ABOUT: usize = 9;

/// 与 [`tab_bar`] 的标签顺序一致；新增页面时必须同步加常量与文案。
pub const TABS: [&str; 10] = [
    "跑步",
    "AI运动",
    "运动记录",
    "数据",
    "我的",
    "设备信息",
    "运行日志",
    "路网",
    "配置档位",
    "关于",
];

/// 顶部标签栏。
///
/// 手机端用**单行横向滚动**（类似主流 App 的顶部 tab），而不是多行网格：
/// 多行网格会占掉近半屏高度，把真正的内容挤下去。
/// 选中项变化时自动滚动到可见区域。
pub fn tab_bar(ui: &mut egui::Ui, selected: &mut usize) -> Vec<egui::Response> {
    let compact = compact_ui(ui);
    let mut responses = Vec::with_capacity(TABS.len());

    if compact {
        // 每格宽度：至少能放下 4 个字（配置档位/设备信息），否则文字会被裁切。
        let avail = ui.available_width();
        let width = ((avail / 4.0) - 1.0).clamp(88.0, 120.0);
        let scroll_id = ui.make_persistent_id("tab_bar_scroll");
        egui::ScrollArea::horizontal()
            .id_salt(scroll_id)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .auto_shrink([false, true])
            // 关键：滚动区必须占满可用宽度，否则内容会被父容器的
            // set_width 夹住，首尾标签被裁掉（实测「设备信息」「关于」被切）。
            .max_width(avail)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (index, label) in TABS.iter().enumerate() {
                        let response = tab_item(
                            ui,
                            egui::vec2(width, TOUCH_HEIGHT),
                            label,
                            *selected == index,
                        );
                        if response.clicked() {
                            *selected = index;
                            // 点击后把该项滚到可见区域，避免选了却看不见。
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        responses.push(response);
                    }
                });
            });
        return responses;
    }

    // 桌面端按文字宽度排布，行内换行兜底，窄窗口下也不会溢出。
    ui.horizontal_wrapped(|ui| {
        for (index, label) in TABS.into_iter().enumerate() {
            let text_width = ui
                .painter()
                .layout_no_wrap(
                    label.to_owned(),
                    egui::FontId::proportional(theme::FONT_BODY),
                    theme::TEXT_SECONDARY,
                )
                .size()
                .x;
            let width = text_width + theme::GAP_MD * 2.0;
            let response = tab_item(ui, egui::vec2(width, 30.0), label, *selected == index);
            if response.clicked() {
                *selected = index;
            }
            responses.push(response);
        }
    });
    responses
}

#[cfg(target_os = "android")]
fn input_width(ui: &egui::Ui, desired: f32) -> f32 {
    desired.min(ui.available_width().max(80.0))
}

#[cfg(target_os = "android")]
fn native_text_button(
    ui: &mut egui::Ui,
    id: egui::Id,
    value: &mut String,
    kind: InputKind,
    desired_width: f32,
) -> egui::Response {
    if let Some(edited) = crate::platform::take_edited_text(id.value()) {
        *value = edited;
    }
    let shown = match kind {
        InputKind::Password if value.is_empty() => "点击输入密码".to_owned(),
        InputKind::Password => "●".repeat(value.chars().count().min(16)),
        _ if value.is_empty() => "点击输入".to_owned(),
        _ => value.clone(),
    };
    let response = ui.add_sized(
        [input_width(ui, desired_width), TOUCH_HEIGHT],
        egui::Button::new(shown),
    );
    if response.clicked() {
        crate::platform::edit_text(id.value(), value, kind);
    }
    response
}

pub fn text_edit(
    ui: &mut egui::Ui,
    id_source: impl Hash,
    value: &mut String,
    kind: InputKind,
    desired_width: f32,
) -> egui::Response {
    let id = ui.make_persistent_id(id_source);
    #[cfg(target_os = "android")]
    {
        native_text_button(ui, id, value, kind, desired_width)
    }
    #[cfg(not(target_os = "android"))]
    {
        let mut edit = egui::TextEdit::singleline(value)
            .id(id)
            .desired_width(desired_width);
        if matches!(kind, InputKind::Password) {
            edit = edit.password(true);
        }
        ui.add(edit)
    }
}

#[cfg(target_os = "android")]
fn show_input_error(ui: &mut egui::Ui, id: egui::Id) {
    if let Some(error) = ui.ctx().data(|data| data.get_temp::<String>(id)) {
        ui.colored_label(crate::ui::theme::err(), error);
    }
}

#[cfg(target_os = "android")]
fn set_input_error(ctx: &egui::Context, id: egui::Id, error: Option<String>) {
    ctx.data_mut(|data| match error {
        Some(error) => data.insert_temp(id, error),
        None => data.remove::<String>(id),
    });
}

#[cfg(any(target_os = "android", test))]
fn numeric_control_shell<R>(
    ui: &mut egui::Ui,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let width = width.min(ui.max_rect().width().max(0.0));
    ui.allocate_ui_with_layout(
        egui::vec2(width, TOUCH_HEIGHT),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            add_contents(ui)
        },
    )
}

pub fn drag_i64(
    ui: &mut egui::Ui,
    id_source: impl Hash,
    value: &mut i64,
    range: RangeInclusive<i64>,
    speed: f64,
    prefix: &str,
    suffix: &str,
) -> egui::Response {
    #[cfg(target_os = "android")]
    {
        let _ = speed;
        let id = ui.make_persistent_id(id_source);
        let error_id = id.with("error");
        let inner = numeric_control_shell(ui, 120.0, |ui| {
            if let Some(edited) = crate::platform::take_edited_text(id.value()) {
                match parse_i64_in_range(&edited, range.clone()) {
                    Ok(parsed) => {
                        *value = parsed;
                        set_input_error(ui.ctx(), error_id, None);
                    }
                    Err(error) => set_input_error(ui.ctx(), error_id, Some(error)),
                }
            }
            let shown = format!("{prefix}{value}{suffix}");
            let response = ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new(shown));
            if response.clicked() {
                crate::platform::edit_text(id.value(), &value.to_string(), InputKind::Integer);
            }
            show_input_error(ui, error_id);
            response
        });
        inner.inner
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = id_source;
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .speed(speed)
                .prefix(prefix)
                .suffix(suffix),
        )
    }
}

pub fn drag_f32(
    ui: &mut egui::Ui,
    id_source: impl Hash,
    value: &mut f32,
    range: RangeInclusive<f32>,
    speed: f64,
    decimals: usize,
    suffix: &str,
) -> egui::Response {
    #[cfg(target_os = "android")]
    {
        let _ = speed;
        let id = ui.make_persistent_id(id_source);
        let error_id = id.with("error");
        let inner = numeric_control_shell(ui, 120.0, |ui| {
            if let Some(edited) = crate::platform::take_edited_text(id.value()) {
                match parse_f32_in_range(&edited, range.clone()) {
                    Ok(parsed) => {
                        *value = parsed;
                        set_input_error(ui.ctx(), error_id, None);
                    }
                    Err(error) => set_input_error(ui.ctx(), error_id, Some(error)),
                }
            }
            let shown = format!("{:.*}{suffix}", decimals, *value);
            let response = ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new(shown));
            if response.clicked() {
                crate::platform::edit_text(id.value(), &value.to_string(), InputKind::Decimal);
            }
            show_input_error(ui, error_id);
            response
        });
        inner.inner
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = id_source;
        let _ = decimals;
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .speed(speed)
                .suffix(suffix),
        )
    }
}

pub fn drag_f64(
    ui: &mut egui::Ui,
    id_source: impl Hash,
    value: &mut f64,
    range: RangeInclusive<f64>,
    speed: f64,
    decimals: usize,
) -> egui::Response {
    #[cfg(target_os = "android")]
    {
        let _ = speed;
        let id = ui.make_persistent_id(id_source);
        let error_id = id.with("error");
        let inner = numeric_control_shell(ui, 150.0, |ui| {
            if let Some(edited) = crate::platform::take_edited_text(id.value()) {
                match parse_f64_in_range(&edited, range.clone()) {
                    Ok(parsed) => {
                        *value = parsed;
                        set_input_error(ui.ctx(), error_id, None);
                    }
                    Err(error) => set_input_error(ui.ctx(), error_id, Some(error)),
                }
            }
            let shown = format!("{:.*}", decimals, *value);
            let response = ui.add_sized([150.0, TOUCH_HEIGHT], egui::Button::new(shown));
            if response.clicked() {
                crate::platform::edit_text(id.value(), &value.to_string(), InputKind::Decimal);
            }
            show_input_error(ui, error_id);
            response
        });
        inner.inner
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = id_source;
        let _ = range;
        ui.add(
            egui::DragValue::new(value)
                .speed(speed)
                .fixed_decimals(decimals),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn balanced_chunks_split_ten_tabs_into_even_rows() {
        // 10 项分 3 行：4/3/3，末行不会只剩 2 项。
        let rows = balanced_chunks(10, 3);
        assert_eq!(rows, vec![0..4, 4..7, 7..10]);
        // 边界：整除时不产生空行，余数全部落在前面的行。
        assert_eq!(balanced_chunks(9, 3), vec![0..3, 3..6, 6..9]);
        assert_eq!(balanced_chunks(2, 3), vec![0..1, 1..2, 2..2]);
        // 每项只出现一次且顺序不变。
        let all: Vec<usize> = rows.iter().flat_map(|r| r.clone()).collect();
        assert_eq!(all, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn compact_breakpoint_covers_phone_widths_but_not_desktop() {
        assert!(is_compact(360.0));
        assert!(is_compact(412.0));
        assert!(!is_compact(880.0));
    }

    /// 手机物理宽度必须经过密度换算后才会进入 compact 布局。
    ///
    /// 这是 APK 布局错乱的根因：egui 默认 pixels_per_point = 1.0，
    /// 1080px 宽的屏幕被当成 1080 逻辑像素，永远大于 620，于是走桌面布局。
    #[test]
    fn physical_width_needs_density_scaling_to_become_compact() {
        // 常见手机：1080×2400，密度 2.75（≈393dp）
        let physical_w = 1080.0f32;
        let density = 2.75f32;
        assert!(!is_compact(physical_w), "未缩放时会被误判为桌面宽屏");
        assert!(
            is_compact(physical_w / density),
            "按 density 缩放后应进入 compact 布局：{}dp",
            physical_w / density
        );
        // 另一组：1080×2340，密度 3.0（360dp）
        assert!(is_compact(1080.0 / 3.0));
        // 平板横屏 2560px / 2.0 = 1280dp → 仍走桌面布局，符合预期
        assert!(!is_compact(2560.0 / 2.0));
    }

    #[test]
    fn integer_input_rejects_invalid_or_out_of_range_without_a_replacement() {
        assert_eq!(parse_i64_in_range(" 23 ", 0..=23), Ok(23));
        assert!(parse_i64_in_range("24", 0..=23).is_err());
        assert!(parse_i64_in_range("abc", 0..=23).is_err());
    }

    #[test]
    fn decimal_input_rejects_non_finite_and_out_of_range_values() {
        assert_eq!(parse_f32_in_range(" 3.25 ", 0.5..=20.0), Ok(3.25));
        assert!(parse_f32_in_range("NaN", 0.5..=20.0).is_err());
        assert!(parse_f32_in_range("21", 0.5..=20.0).is_err());
    }

    #[test]
    fn tab_bar_stays_inside_offscreen_phone_and_desktop_frames() {
        for width in [360.0, 412.0, 880.0] {
            let ctx = egui::Context::default();
            let mut rects = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut selected = 0;
                    rects = tab_bar(ui, &mut selected)
                        .into_iter()
                        .map(|response| response.rect)
                        .collect();
                });
            });

            assert_eq!(rects.len(), 10, "width={width}");
            assert!(rects
                .iter()
                .all(|rect| rect.is_finite() && rect.width() > 0.0));
            if is_compact(width) {
                // 手机端是横向滚动条：标签按顺序向右延伸、允许超出屏幕宽度，
                // 但不能出现竖直方向的换行（所有 rect 的 y 必须相同）。
                let y0 = rects[0].min.y;
                assert!(
                    rects.iter().all(|r| (r.min.y - y0).abs() < 0.5),
                    "width={width}: 标签不应换行 {rects:?}"
                );
                assert!(
                    rects.iter().all(|rect| rect.height() >= 44.0),
                    "width={width}: {rects:?}"
                );
                // 相邻标签水平紧邻、不重叠
                for pair in rects.windows(2) {
                    assert!(
                        pair[1].min.x >= pair[0].max.x - 0.5,
                        "width={width}: 标签重叠 {pair:?}"
                    );
                }
            } else {
                assert!(
                    rects.iter().all(|rect| rect.max.x <= width + 0.5),
                    "width={width}: {rects:?}"
                );
            }
        }
    }

    #[test]
    fn responsive_parameter_row_wraps_without_invalid_or_overflowing_rectangles() {
        for width in [360.0, 412.0, 880.0] {
            let ctx = egui::Context::default();
            let mut rects = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    row(ui, |ui| {
                        for (index, control_width) in
                            [110.0, 120.0, 90.0, 120.0].into_iter().enumerate()
                        {
                            rects.push(
                                ui.add_sized(
                                    [control_width, TOUCH_HEIGHT],
                                    egui::Button::new(format!("参数 {index}")),
                                )
                                .rect,
                            );
                        }
                    });
                });
            });

            assert_eq!(rects.len(), 4, "width={width}");
            assert!(rects
                .iter()
                .all(|rect| rect.is_finite() && rect.is_positive()));
            assert!(
                rects.iter().all(|rect| rect.max.x <= width + 0.5),
                "width={width}: {rects:?}"
            );
        }
    }

    #[test]
    fn numeric_errors_expand_rows_without_overlapping_following_controls() {
        for width in [320.0, 360.0, 412.0] {
            let ctx = egui::Context::default();
            let mut rectangles = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    row(ui, |ui| {
                        for index in 0..3 {
                            let control = numeric_control_shell(ui, 120.0, |ui| {
                                let button = ui
                                    .add_sized([120.0, TOUCH_HEIGHT], egui::Button::new("360"))
                                    .rect;
                                let error = ui
                                    .colored_label(
                                        egui::Color32::RED,
                                        if index == 0 {
                                            "Please enter a valid value between 10 and 200"
                                        } else {
                                            "请输入 10 到 200"
                                        },
                                    )
                                    .rect;
                                (button, error)
                            });
                            assert!(control.response.rect.contains_rect(control.inner.0));
                            assert!(control.response.rect.contains_rect(control.inner.1));
                            rectangles.push(control.response.rect);
                        }
                    });
                    rectangles.push(ui.button("Next row").rect);
                });
            });
            for (index, rectangle) in rectangles.iter().enumerate() {
                assert!(
                    rectangle.max.x <= width + 0.5,
                    "width={width}: {rectangles:?}"
                );
                for other in &rectangles[index + 1..] {
                    assert!(
                        !rectangle.intersects(*other),
                        "width={width}: {rectangles:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn wrapped_range_controls_reserve_width_before_the_parent_places_them() {
        for width in [360.0, 412.0] {
            let ctx = egui::Context::default();
            let mut rects = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    row(ui, |ui| {
                        rects.push(ui.label("配速范围（秒/km）：").rect);
                        let first = numeric_control_shell(ui, 120.0, |ui| {
                            ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new("360"))
                        });
                        rects.push(first.inner.rect);
                        rects.push(ui.label("至").rect);
                        let second = numeric_control_shell(ui, 120.0, |ui| {
                            ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new("480"))
                        });
                        rects.push(second.inner.rect);
                    });
                });
            });

            assert!(rects
                .iter()
                .all(|rect| rect.is_finite() && rect.is_positive()));
            assert!(
                rects.iter().all(|rect| rect.max.x <= width + 0.5),
                "width={width}: {rects:?}"
            );
        }
    }
}
