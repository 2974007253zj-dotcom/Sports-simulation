//! 运动记录页：跑步 / AI 两个分栏，egui_extras 表格。

use super::{mobile, theme, App};
use chrono::{NaiveDate, TimeZone};
use eframe::egui;
use egui_extras::{Column, TableBuilder};

/// 学校学期划分：一年两个学期，按日期落在哪一段就算哪个学期。
///
/// * 下学期：2 月 16 日 – 8 月 15 日
/// * 上学期：8 月 16 日 – 次年 2 月 15 日
///
/// 两个区间首尾相接且不重叠，任何日期都有唯一归属。
pub struct Semester {
    /// 学期起始日（本地时间 00:00:00）的毫秒时间戳。
    pub start_ms: i64,
    /// 学期结束日的**次日** 00:00 的毫秒时间戳（左闭右开）。
    pub end_ms: i64,
    /// 学期名称，如「2026-2027 上学期」。
    pub name: String,
}

impl Semester {
    /// 按给定日期算出所属学期。
    pub fn containing(date: chrono::NaiveDate) -> Self {
        use chrono::{Datelike, TimeZone};
        let (y, m, d) = (date.year(), date.month(), date.day());
        // 2/16–8/15 是同一个自然年内的下学期；8/16 起跨年到次年 2/15。
        let in_spring = (m == 2 && d >= 16) || (3..=7).contains(&m) || (m == 8 && d <= 15);
        let (start, end, name) = if in_spring {
            (
                NaiveDate::from_ymd_opt(y, 2, 16).unwrap(),
                // 结束日 8/15，右边界取 8/16
                NaiveDate::from_ymd_opt(y, 8, 16).unwrap(),
                format!("{y} 下学期（2/16–8/15）"),
            )
        } else {
            // 8/16 及以后属于 y~y+1 学期；1/1–2/15 属于 y-1~y 学期。
            let first_year = if m == 1 || (m == 2 && d <= 15) { y - 1 } else { y };
            (
                NaiveDate::from_ymd_opt(first_year, 8, 16).unwrap(),
                NaiveDate::from_ymd_opt(first_year + 1, 2, 16).unwrap(),
                format!("{first_year}-{} 上学期（8/16–次年2/15）", first_year + 1),
            )
        };
        let start_ms = chrono::Local
            .from_local_datetime(&start.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .map(|t| t.timestamp_millis())
            .unwrap_or(0);
        let end_ms = chrono::Local
            .from_local_datetime(&end.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .map(|t| t.timestamp_millis())
            .unwrap_or(i64::MAX);
        Self {
            start_ms,
            end_ms,
            name,
        }
    }

    /// 今天所属的学期。
    pub fn current() -> Self {
        Self::containing(chrono::Local::now().date_naive())
    }
}

/// 记录视图只显示**本学期**（范围由 [`Semester::current`] 按日期自动计算）。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum RecordsWindow {
    #[default]
    Semester,
}

impl RecordsWindow {
    pub fn label(self) -> &'static str {
        "本学期"
    }

    /// 时间范围（左闭右开）。
    pub fn range_ms(self) -> (i64, i64) {
        let s = Semester::current();
        (s.start_ms, s.end_ms)
    }
}

#[derive(Default)]
pub struct RecordsPage {
    pub rows: Vec<crate::api::records::RecordRow>,
    pub ai_groups: Vec<crate::api::ai::AiRecordGroup>,
    pub ai_total: i64,
    pub sub: usize,
    /// 被服务端标记（status=1/2）的记录数。
    pub flagged: usize,
    /// 当前展开详情的 rrid
    pub detail_rrid: Option<i64>,
    pub detail_raw: Option<serde_json::Value>,
    pub detail_loading: bool,
    /// AI 详情页当前记录
    pub ai_detail_id: Option<i64>,
    pub ai_detail_raw: Option<serde_json::Value>,
    pub ai_detail_loading: bool,
}

impl App {
    pub fn draw_records(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        mobile::row(ui, |ui| {
            ui.selectable_value(&mut self.records_page.sub, 0, "跑步");
            ui.selectable_value(&mut self.records_page.sub, 1, "AI 运动");
            ui.add_space(8.0);
            match self.records_page.sub {
                0 => {
                    if ui
                        .add_enabled(!self.records_busy, theme::primary_btn("刷新"))
                        .clicked()
                    {
                        self.refresh_records();
                    }
                    if self.records_busy {
                        ui.label("拉取中…");
                    }
                    ui.separator();
                    // 只显示本学期，范围后台按日期自动计算，不给切换入口。
                    ui.label(
                        egui::RichText::new(Semester::current().name)
                            .small()
                            .color(theme::text_dim()),
                    );
                    if self.records_page.flagged > 0 {
                        ui.colored_label(
                            theme::err(),
                            format!("{} 条被标记", self.records_page.flagged),
                        );
                    }
                }
                _ => {
                    if ui
                        .add_enabled(!self.records_busy, theme::primary_btn("刷新全部记录"))
                        .clicked()
                    {
                        self.refresh_ai_records();
                    }
                    if self.records_busy {
                        ui.label("拉取中…");
                    }
                }
            }
        });
        ui.separator();

        match self.records_page.sub {
            0 => self.draw_run_view(ui),
            _ => self.draw_ai_view(ui),
        }
    }

    /// AI 视图：列表与详情两个页面互斥。
    fn draw_ai_view(&mut self, ui: &mut egui::Ui) {
        if self.records_page.ai_detail_id.is_some() {
            self.draw_ai_detail_view(ui);
        } else {
            self.draw_ai_table(ui);
        }
    }

    /// AI 详情独立页：返回 + 全量字段。
    fn draw_ai_detail_view(&mut self, ui: &mut egui::Ui) {
        let id = self.records_page.ai_detail_id.unwrap_or(0);
        ui.add_space(4.0);
        mobile::row(ui, |ui| {
            if ui.button("← 返回列表").clicked() {
                self.records_page.ai_detail_id = None;
                self.records_page.ai_detail_raw = None;
                self.records_page.ai_detail_loading = false;
            }
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!("AI 运动详情 #{}", id))
                    .strong()
                    .color(theme::accent()),
            );
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.records_page.ai_detail_loading && self.records_page.ai_detail_raw.is_none()
                {
                    ui.label("加载详情中…");
                } else if let Some(raw) = &self.records_page.ai_detail_raw {
                    if let Some(err) = raw.get("fetchError").and_then(|x| x.as_str()) {
                        ui.colored_label(theme::err(), err);
                    } else {
                        draw_ai_detail_panel(ui, raw);
                    }
                } else {
                    ui.label("（无详情数据）");
                }
            });
    }

    /// 跑步视图：列表与详情两个页面互斥。
    fn draw_run_view(&mut self, ui: &mut egui::Ui) {
        if self.records_page.detail_rrid.is_some() {
            self.draw_detail_view(ui);
        } else {
            self.draw_run_table(ui);
        }
    }

    /// 详情独立页：返回 + 全部字段。
    fn draw_detail_view(&mut self, ui: &mut egui::Ui) {
        let rrid = self.records_page.detail_rrid.unwrap_or(0);
        ui.add_space(4.0);
        mobile::row(ui, |ui| {
            if ui.button("← 返回列表").clicked() {
                self.records_page.detail_rrid = None;
                self.records_page.detail_raw = None;
                self.records_page.detail_loading = false;
            }
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!("跑步详情 #{}", rrid))
                    .strong()
                    .color(theme::accent()),
            );
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.records_page.detail_loading && self.records_page.detail_raw.is_none() {
                    ui.label("加载详情中…");
                } else if let Some(raw) = &self.records_page.detail_raw {
                    draw_detail_panel(ui, raw);
                } else {
                    ui.label("（无详情数据）");
                }
            });
    }

    fn draw_run_table(&mut self, ui: &mut egui::Ui) {
        if self.records_page.rows.is_empty() {
            ui.add_space(20.0);
            ui.centered_and_justified(|ui| ui.label("暂无跑步记录"));
            return;
        }
        // 时间范围过滤：默认只看本学期（6 个月）。服务端一次性给 100 条，
        // 学期外的记录默认收起，但切到「全部」随时能看。
        // 只保留本学期：[学期开始日 00:00, 结束日次日 00:00)。
        let sem = Semester::current();
        let (start, end) = RecordsWindow::Semester.range_ms();
        let rows: Vec<crate::api::records::RecordRow> = self
            .records_page
            .rows
            .iter()
            .filter(|r| r.start_time >= start && r.start_time < end)
            .cloned()
            .collect();
        if rows.is_empty() {
            ui.add_space(20.0);
            ui.centered_and_justified(|ui| {
                ui.label(format!("{}内暂无跑步记录", sem.name))
            });
            return;
        }
        ui.label(
            egui::RichText::new(format!("{} · {} 条", sem.name, rows.len()))
                .color(theme::text_dim()),
        );
        let fmt_time = |ms: i64| -> String {
            chrono::Local
                .timestamp_millis_opt(ms)
                .single()
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default()
        };
        let fmt_dur = |s: i64| -> String {
            if s >= 3600 {
                format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
            } else {
                format!("{}:{:02}", s / 60, s % 60)
            }
        };
        let pace_of = |dis: f64, t: i64| -> (i64, i64) {
            let p = if dis > 0.0 {
                t as f64 / (dis / 1000.0)
            } else {
                0.0
            };
            ((p / 60.0) as i64, (p as i64) % 60)
        };

        let toggle_target = std::cell::Cell::new(None::<i64>);

        if mobile::compact_ui(ui) {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for r in &rows {
                        let rrid = r.rrid;
                        let (pace_min, pace_sec) = pace_of(r.total_dis, r.total_time);
                        theme::card_frame().show(ui, |ui| {
                            mobile::row(ui, |ui| {
                                ui.strong(fmt_time(r.start_time));
                                if r.is_flagged() {
                                    ui.colored_label(theme::err(), if r.status == 2 { "已判违规" } else { "被标记" });
                                } else if r.complete {
                                    ui.colored_label(theme::ok(), "达标");
                                } else {
                                    ui.colored_label(theme::err(), "未达标");
                                }
                            });
                            if r.is_flagged() && !r.appeal_note().is_empty() {
                                ui.colored_label(theme::err(), r.appeal_note());
                            }
                            ui.label(format!(
                                "{:.2} km · {} · 配速 {pace_min}:{pace_sec:02}/km",
                                r.total_dis / 1000.0,
                                fmt_dur(r.total_time),
                            ));
                            ui.label(format!("步频 {} spm · rrid {rrid}", r.avg_step_freq));
                            if ui
                                .add_sized([96.0, mobile::TOUCH_HEIGHT], egui::Button::new("详情"))
                                .clicked()
                            {
                                toggle_target.set(Some(rrid));
                            }
                        });
                        ui.add_space(6.0);
                    }
                });
            if let Some(rrid) = toggle_target.get() {
                self.records_page.detail_rrid = Some(rrid);
                self.records_page.detail_raw = None;
                self.records_page.detail_loading = true;
                self.fetch_run_detail(rrid);
            }
            return;
        }

        TableBuilder::new(ui)
            .striped(true)
            .vscroll(true)
            .auto_shrink([false, false])
            .column(Column::auto().at_least(130.0)) // 时间
            .column(Column::auto().at_least(70.0)) // 距离
            .column(Column::auto().at_least(70.0)) // 时长
            .column(Column::auto().at_least(60.0)) // 配速
            .column(Column::auto().at_least(70.0)) // 步频
            .column(Column::auto().at_least(60.0)) // 达标
            .column(Column::auto().at_least(90.0)) // rrid
            .column(Column::remainder()) // 操作
            .header(22.0, |mut h| {
                for t in [
                    "时间", "距离", "时长", "配速", "步频", "判定", "rrid", "操作",
                ] {
                    h.col(|ui| {
                        ui.label(egui::RichText::new(t).strong().color(theme::text()));
                    });
                }
            })
            .body(|mut body| {
                let sel = self.records_page.detail_rrid;
                for r in &rows {
                    let rrid = r.rrid;
                    let is_detail = sel == Some(rrid);
                    body.row(20.0, |mut row| {
                        row.col(|ui| {
                            let color = if is_detail {
                                theme::accent()
                            } else {
                                theme::text()
                            };
                            ui.label(egui::RichText::new(fmt_time(r.start_time)).color(color));
                        });
                        row.col(|ui| {
                            ui.monospace(format!("{:.2} km", r.total_dis / 1000.0));
                        });
                        row.col(|ui| {
                            ui.monospace(fmt_dur(r.total_time));
                        });
                        row.col(|ui| {
                            let (m, s) = pace_of(r.total_dis, r.total_time);
                            ui.monospace(format!("{m}:{s:02}"));
                        });
                        row.col(|ui| {
                            ui.monospace(format!("{} spm", r.avg_step_freq));
                        });
                        row.col(|ui| {
                            if r.is_flagged() {
                                ui.colored_label(
                                    theme::err(),
                                    if r.status == 2 { "已判违规" } else { "被标记" },
                                );
                            } else if r.complete {
                                ui.colored_label(theme::ok(), "达标");
                            } else {
                                ui.colored_label(theme::err(), "未达标");
                            }
                        });
                        row.col(|ui| {
                            ui.monospace(rrid.to_string());
                        });
                        row.col(|ui| {
                            let txt = if is_detail {
                                egui::RichText::new("收起").color(theme::accent())
                            } else {
                                egui::RichText::new("详情").color(theme::accent())
                            };
                            if ui.small_button(txt).clicked() {
                                toggle_target.set(if is_detail { None } else { Some(rrid) });
                            }
                        });
                    });
                }
            });

        if let Some(rrid) = toggle_target.get() {
            self.records_page.detail_rrid = Some(rrid);
            self.records_page.detail_raw = None;
            self.records_page.detail_loading = true;
            self.fetch_run_detail(rrid);
        }
    }

    fn draw_ai_table(&mut self, ui: &mut egui::Ui) {
        if self.records_page.ai_groups.is_empty() {
            ui.add_space(20.0);
            ui.centered_and_justified(|ui| ui.label("暂无 AI 记录，选择项目后「刷新记录」"));
            return;
        }
        ui.label(format!(
            "共 {} 条（{} 天）",
            self.records_page.ai_total,
            self.records_page.ai_groups.len()
        ));
        let num = |s: &str| s.parse::<f64>().unwrap_or(0.0);
        let toggle_target = std::cell::Cell::new(None::<i64>);
        if mobile::compact_ui(ui) {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for g in &self.records_page.ai_groups {
                        let date = chrono::Local
                            .timestamp_millis_opt(g.score_date)
                            .single()
                            .map(|t| t.format("%Y-%m-%d").to_string())
                            .unwrap_or_default();
                        for r in &g.records {
                            theme::card_frame().show(ui, |ui| {
                                ui.strong(format!("{} · {}", date, r.name));
                                let score = if r.rtype == 2 {
                                    format!("{:.1} 秒", num(&r.score) / 1000.0)
                                } else {
                                    format!("{} 个", r.score)
                                };
                                ui.label(format!(
                                    "成绩 {score} · 视频 {}",
                                    if r.has_video { "有" } else { "-" }
                                ));
                                ui.label(format!(
                                    "提交 {}",
                                    chrono::Local
                                        .timestamp_millis_opt(r.upload_time)
                                        .single()
                                        .map(|t| t.format("%m-%d %H:%M").to_string())
                                        .unwrap_or_default(),
                                ));
                                if ui
                                    .add_sized(
                                        [96.0, mobile::TOUCH_HEIGHT],
                                        egui::Button::new("详情"),
                                    )
                                    .clicked()
                                {
                                    toggle_target.set(Some(r.id));
                                }
                            });
                            ui.add_space(6.0);
                        }
                    }
                });
            if let Some(id) = toggle_target.get() {
                self.records_page.ai_detail_id = Some(id);
                self.records_page.ai_detail_raw = None;
                self.records_page.ai_detail_loading = true;
                self.fetch_ai_detail(id);
            }
            return;
        }
        TableBuilder::new(ui)
            .striped(true)
            .vscroll(true)
            .auto_shrink([false, false])
            .column(Column::auto().at_least(100.0)) // 日期
            .column(Column::auto().at_least(110.0)) // 项目
            .column(Column::auto().at_least(90.0)) // 成绩
            .column(Column::auto().at_least(100.0)) // 完成时间
            .column(Column::auto().at_least(90.0)) // 提交时间
            .column(Column::auto().at_least(60.0)) // 视频
            .column(Column::remainder()) // 操作
            .header(22.0, |mut h| {
                for t in [
                    "日期",
                    "项目",
                    "成绩",
                    "完成时间",
                    "提交时间",
                    "视频",
                    "操作",
                ] {
                    h.col(|ui| {
                        ui.label(egui::RichText::new(t).strong().color(theme::text()));
                    });
                }
            })
            .body(|mut body| {
                for g in &self.records_page.ai_groups {
                    let date = chrono::Local
                        .timestamp_millis_opt(g.score_date)
                        .single()
                        .map(|t| t.format("%Y-%m-%d").to_string())
                        .unwrap_or_default();
                    for r in &g.records {
                        body.row(20.0, |mut row| {
                            row.col(|ui| {
                                ui.monospace(&date);
                            });
                            row.col(|ui| {
                                ui.label(&r.name);
                            });
                            row.col(|ui| {
                                if r.rtype == 2 {
                                    // 计时类：成绩即用时（毫秒）
                                    ui.monospace(format!("{:.1} 秒", num(&r.score) / 1000.0));
                                } else {
                                    ui.monospace(format!("{} 个", r.score));
                                }
                            });
                            row.col(|ui| {
                                let ms = chrono::Local
                                    .timestamp_millis_opt(r.score_date)
                                    .single()
                                    .map(|t| t.format("%H:%M:%S").to_string())
                                    .unwrap_or_default();
                                ui.monospace(ms);
                            });
                            row.col(|ui| {
                                ui.monospace(
                                    chrono::Local
                                        .timestamp_millis_opt(r.upload_time)
                                        .single()
                                        .map(|t| t.format("%m-%d %H:%M").to_string())
                                        .unwrap_or_default(),
                                );
                            });
                            row.col(|ui| {
                                if r.has_video {
                                    ui.colored_label(theme::accent(), "有");
                                } else {
                                    ui.label("-");
                                }
                            });
                            row.col(|ui| {
                                if ui.small_button("详情").clicked() {
                                    toggle_target.set(Some(r.id));
                                }
                            });
                        });
                    }
                }
            });

        if let Some(id) = toggle_target.get() {
            self.records_page.ai_detail_id = Some(id);
            self.records_page.ai_detail_raw = None;
            self.records_page.ai_detail_loading = true;
            self.fetch_ai_detail(id);
        }
    }
}

/// AI 记录详情：record/info 全量字段。
fn draw_ai_detail_panel(ui: &mut egui::Ui, raw: &serde_json::Value) {
    let i = |k: &str| raw.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    let f = |k: &str| raw.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let s = |k: &str| -> String {
        raw.get(k)
            .and_then(|v| v.as_str())
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
            .unwrap_or_else(|| "-".into())
    };
    let dt = |k: &str| -> String {
        let ms = i(k);
        if ms <= 0 {
            return "-".into();
        }
        chrono::Local
            .timestamp_millis_opt(ms)
            .single()
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".into())
    };
    let fmt_ms = |ms: i64| -> String {
        if ms <= 0 {
            return "-".into();
        }
        if ms >= 60_000 {
            format!(
                "{}:{:02}:{:02}",
                ms / 60_000,
                ms % 60_000 / 1000,
                ms % 1000 / 10
            )
        } else {
            format!("{}.{:02} 秒", ms / 1000, ms % 1000 / 10)
        }
    };

    let rtype = i("type");
    // 详情接口的 score 单位与列表不同（计时类为计分制），展示原始值，用时以 timeConsume 为准
    let score_text = if rtype == 2 {
        format!("{}（原始计分）", i("score"))
    } else {
        format!("{} 个", i("score"))
    };

    let mut rows: Vec<(&str, String)> = vec![
        ("项目名称", s("name")),
        (
            "类型",
            if rtype == 2 {
                "计时".into()
            } else {
                "计次".into()
            },
        ),
        ("成绩", score_text),
        ("用时", fmt_ms(i("timeConsume"))),
        ("速度", {
            let sp = f("speed");
            if sp > 0.0 {
                format!("{sp:.0} /分")
            } else {
                "-".into()
            }
        }),
        ("消耗", {
            let c = f("consume");
            if c > 0.0 {
                format!("{c:.1} kcal")
            } else {
                "0 kcal".into()
            }
        }),
        ("完成时间", dt("scoreDate")),
        ("提交时间", dt("uploadTime")),
        ("状态", i("status").to_string()),
        ("记录 ID", i("id").to_string()),
        ("项目 ID", i("sportId").to_string()),
        ("任务 ID", i("taskId").to_string()),
        ("用户 ID", i("uid").to_string()),
        ("记录 UUID", s("uuid")),
        ("视频", s("exerciseMediaUrl")),
    ];
    if let Some(reason) = raw.get("reason").and_then(|v| v.as_str()) {
        if !reason.is_empty() {
            rows.push(("原因", reason.to_string()));
        }
    }
    let draw_value = |ui: &mut egui::Ui, k: &str, v: &String| {
        mobile::row(ui, |ui| {
            ui.label(egui::RichText::new(v).color(theme::text()));
            let copyable = (k == "视频" && v != "-") || k == "记录 UUID";
            if copyable && ui.button("复制").clicked() {
                ui.ctx().copy_text(v.clone());
            }
        });
    };
    if mobile::compact_ui(ui) {
        for (k, v) in &rows {
            theme::card_frame().show(ui, |ui| {
                ui.label(egui::RichText::new(*k).color(theme::text_dim()));
                draw_value(ui, k, v);
            });
            ui.add_space(4.0);
        }
    } else {
        egui::Grid::new("ai_detail_grid")
            .num_columns(2)
            .spacing([18.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                for (k, v) in rows {
                    ui.label(egui::RichText::new(k).color(theme::text_dim()));
                    draw_value(ui, k, &v);
                    ui.end_row();
                }
            });
    }
}

fn draw_detail_panel(ui: &mut egui::Ui, raw: &serde_json::Value) {
    let g = |k: &str| -> String {
        match raw.get(k) {
            Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => String::new(),
        }
    };
    let f = |k: &str| -> f64 { raw.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) };
    let i = |k: &str| -> i64 { raw.get(k).and_then(|v| v.as_i64()).unwrap_or(0) };

    let mut items: Vec<(&str, String)> = vec![
        ("距离", format!("{:.2} km", f("totalDis") / 1000.0)),
        ("有效里程", format!("{:.2} km", f("validDis") / 1000.0)),
        ("时长", {
            let t = i("totalTime");
            if t >= 3600 {
                format!("{}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
            } else {
                format!("{}:{:02}", t / 60, t % 60)
            }
        }),
        ("卡路里", format!("{} kcal", i("calorie"))),
        ("功率", format!("{} W", i("avgPower"))),
        ("步数", i("totalSteps").to_string()),
        ("步频", format!("{} spm", i("avgStepFreq"))),
        ("爬升", format!("{} m", i("totalAscent"))),
        ("配速区间", {
            let lo = f("speedBottom");
            let hi = f("speedTop");
            if hi > 0.0 {
                format!("{:.2} ~ {:.2} m/s", lo, hi)
            } else {
                "-".into()
            }
        }),
        ("地址", g("address")),
        ("状态", g("statusInfo")),
        ("判定", if i("isCheat") == 1 { "已被系统判为可疑".into() } else { "正常".into() }),
        ("申诉", match i("appealStatus") {
            1 => "可申诉".into(),
            2 => "申诉中".into(),
            3 => "申诉通过".into(),
            4 => format!("申诉未通过{}", if g("appealStatusInfo").is_empty() { String::new() } else { format!("（{}）", g("appealStatusInfo")) }),
            _ => String::new(),
        }),
    ];
    let st_ms = i("startTime");
    if st_ms > 0 {
        let st = chrono::Local
            .timestamp_millis_opt(st_ms)
            .single()
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default();
        if !st.is_empty() {
            items.push(("开始时间", st));
        }
    }
    if mobile::compact_ui(ui) {
        for (k, v) in &items {
            if v.is_empty() || v == "0" || v == "0.0" {
                continue;
            }
            theme::card_frame().show(ui, |ui| {
                ui.label(egui::RichText::new(*k).color(theme::text_dim()));
                ui.label(egui::RichText::new(v).color(theme::text()));
            });
            ui.add_space(4.0);
        }
    } else {
        egui::Grid::new("detail_grid")
            .num_columns(2)
            .spacing([18.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                for (k, v) in &items {
                    if v.is_empty() || v == "0" || v == "0.0" {
                        continue;
                    }
                    ui.label(egui::RichText::new(*k).color(theme::text_dim()));
                    ui.label(egui::RichText::new(v).color(theme::text()));
                    ui.end_row();
                }
            });
    }

    // 达标判定
    if let Some(list) = raw.get("reasonList").and_then(|x| x.as_array()) {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("达标判定")
                .strong()
                .color(theme::text()),
        );
        for r in list {
            let ok = r.get("complete").and_then(|x| x.as_bool()).unwrap_or(false);
            let reason = r.get("reason").and_then(|x| x.as_str()).unwrap_or("");
            mobile::row(ui, |ui| {
                if ok {
                    ui.colored_label(theme::ok(), "√");
                } else {
                    ui.colored_label(theme::err(), "×");
                }
                ui.label(reason);
            });
        }
    }
}

#[cfg(test)]
mod semester_tests {
    use super::Semester;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn august_16_starts_the_autumn_semester() {
        // 8/15 仍属于当年下学期，8/16 起进入跨年上学期——这是唯一的切换点。
        let a = Semester::containing(d(2026, 8, 15));
        assert!(a.name.contains("2026 下学期"), "{}", a.name);

        let b = Semester::containing(d(2026, 8, 16));
        assert!(b.name.contains("2026-2027 上学期"), "{}", b.name);
    }

    #[test]
    fn february_16_starts_the_spring_semester() {
        let a = Semester::containing(d(2027, 2, 15));
        assert!(a.name.contains("2026-2027 上学期"), "{}", a.name);

        let b = Semester::containing(d(2027, 2, 16));
        assert!(b.name.contains("2027 下学期"), "{}", b.name);
    }

    #[test]
    fn today_2026_11_02_is_in_the_autumn_semester() {
        let s = Semester::containing(d(2026, 11, 2));
        assert!(s.name.contains("2026-2027 上学期"), "{}", s.name);
        // 学期从 2026-08-16 开始，到 2027-02-16 结束（右开）
        let start = chrono::DateTime::from_timestamp_millis(s.start_ms).unwrap();
        let end = chrono::DateTime::from_timestamp_millis(s.end_ms).unwrap();
        let start_local = start.with_timezone(&chrono::Local);
        let end_local = end.with_timezone(&chrono::Local);
        assert_eq!(start_local.format("%Y-%m-%d").to_string(), "2026-08-16");
        assert_eq!(end_local.format("%Y-%m-%d").to_string(), "2027-02-16");
    }

    #[test]
    fn january_belongs_to_the_previous_autumn_semester() {
        let s = Semester::containing(d(2027, 1, 15));
        assert!(s.name.contains("2026-2027 上学期"), "{}", s.name);
    }

    #[test]
    fn every_date_has_exactly_one_semester() {
        // 全年逐日检查：相邻日期间隔恰为 1 天时不跨学期，跨学期时首尾相接。
        let mut date = d(2026, 1, 1);
        for _ in 0..800 {
            let a = Semester::containing(date);
            let next = date.succ_opt().unwrap();
            let b = Semester::containing(next);
            if a.name == b.name {
                assert_eq!(a.start_ms, b.start_ms);
                assert_eq!(a.end_ms, b.end_ms);
            } else {
                // 换学期：新学期的开始正好是旧学期的结束
                assert_eq!(a.end_ms, b.start_ms, "{date} 与 {next} 学期不连续");
            }
            assert!(a.start_ms < a.end_ms);
            date = next;
        }
    }
}
