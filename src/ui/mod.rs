//! 界面：横排 / 竖排两种指标卡片（全部自绘），右侧悬浮"设置 / 关闭"两个竖排按钮。
//!
//! - `theme`  配色
//! - `text`   字体与文字绘制
//! - `chips`  单个指标的标签 + 数值
//! - `bar`    卡片版式（测量 + 绘制）与悬浮按钮
//! - `icons`  自绘图标
//! - `window` bar 窗口的位置与尺寸

mod bar;
mod chips;
mod icons;
mod text;
mod theme;
mod window;

pub use bar::card;
pub use icons::{app_icon, check, draw_layout_thumb, switch};
pub use text::{at as text_at, fam, text_w, FONT_R, FONT_SB};
pub use theme::{card_bg, mix, on_accent, page_bg, palette, Palette};
pub use window::{WindowState, BOTTOM_MARGIN, OFFSCREEN};

use crate::config::LayoutMode;
use crate::data::Status;
use crate::App;
use eframe::egui::{self, pos2, CursorIcon, Rect};

pub const BAR_H: f32 = bar::BAR_H;

/// 版本号（设置窗口、更新提示里用），唯一来源是 Cargo.toml
pub fn version() -> String {
    format!("v{}", crate::update::VERSION)
}

/// 打包时间（build.rs 写进 exe 的）
pub fn built_at() -> &'static str {
    crate::update::BUILT_AT
}

/// 用户点了卡片上的按钮
enum Action {
    Close,
    OpenSettings,
}

impl App {
    pub fn draw(&mut self, ctx: &egui::Context) {
        let pal = palette(self.dark);
        let painter = ctx.layer_painter(egui::LayerId::background());
        let time = ctx.input(|i| i.time) as f32;
        let dt = ctx.input(|i| i.stable_dt).min(0.1);

        let metrics = self.cfg.visible_metrics();
        let live = self.snap.status == Status::Generating;
        // 生成中，或者任务还在跑（跑工具/思考）——都算"在干活"，圆点不要变灰
        let active = live || self.snap.active;
        // 新任务页 / 新会话还没开跑：两种版式都用同一张窄空卡
        let empty = self.snap.total.is_none();
        let chips1 = chips::build(self.snap.turn.as_ref(), &metrics, &pal);
        let chips2 = chips::build(self.snap.total.as_ref(), &metrics, &pal);
        let lab2 = total_label(self.snap.stale, self.snap.turns_count);
        let horizontal = self.cfg.layout == LayoutMode::Horizontal;
        let rows = bar::Rows {
            turn: &chips1,
            total: &chips2,
            turn_label: "本轮",
            total_label: &lab2,
        };

        let view = if empty {
            bar::measure_empty(ctx)
        } else if horizontal {
            bar::measure_horizontal(ctx, &rows)
        } else {
            bar::measure_vertical(ctx, &rows)
        };
        let mon = self.monitor_rect(ctx);
        let origin = self
            .win
            .resolve_origin(&mut self.cfg, mon, view, self.place.is_none());
        self.win.apply_size(ctx, view, origin, self.visible);

        let body = Rect::from_min_size(pos2(0.0, 0.0), view);
        bar::card(&painter, body, &pal, 14);
        let render = bar::View {
            rows,
            pal: &pal,
            time,
            live,
            active,
        };
        if empty {
            bar::draw_empty(ctx, &painter, body, &pal, time);
        } else if horizontal {
            bar::draw_horizontal(ctx, &painter, body, &render);
        } else {
            bar::draw_vertical(ctx, &painter, body, &render);
        }

        // ---- 悬浮按钮
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let (gear, close) = bar::hover_buttons(body);
        let over_btn = pointer
            .map(|q| gear.contains(q) || close.contains(q))
            .unwrap_or(false);
        let body_hover = pointer.map(|q| body.contains(q)).unwrap_or(false);
        let target = if body_hover || self.settings_open {
            1.0
        } else {
            0.0
        };
        self.hover_t += (target - self.hover_t) * (dt * 14.0).min(1.0);
        if self.hover_t < 0.004 {
            self.hover_t = 0.0;
        }
        bar::draw_buttons(&painter, &pal, body, pointer, self.hover_t);

        if over_btn {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        } else if body_hover {
            ctx.set_cursor_icon(CursorIcon::Grab);
        }

        // ---- 输入
        let clicked = ctx.input(|i| i.pointer.primary_clicked())
            || ctx.input(|i| i.pointer.primary_released());
        let pressed = ctx.input(|i| i.pointer.primary_pressed());
        // 窗口刚被唤醒时，Windows 会把"激活窗口"的那次按下吞掉，右键抬起也算
        let secondary = ctx.input(|i| i.pointer.secondary_clicked())
            || ctx.input(|i| i.pointer.secondary_released());

        let mut actions: Vec<Action> = Vec::new();
        if let Some(q) = pointer {
            if clicked {
                if close.contains(q) {
                    actions.push(Action::Close);
                } else if gear.contains(q) {
                    actions.push(Action::OpenSettings);
                }
            }
            if secondary && body.contains(q) && !over_btn {
                actions.push(Action::OpenSettings);
            }
        }
        if pressed && body_hover && !over_btn {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        for a in actions {
            match a {
                Action::Close => {
                    self.cfg.save();
                    self.tracker.save_cache();
                    // 有下载好但还没换上去的新版本：走的时候顺手换掉，下次打开就是新版
                    if self.updater.status().is_ready() {
                        self.updater.apply(false);
                    }
                    self.quit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Action::OpenSettings => self.open_settings(ctx),
            }
        }

        if self.visible {
            self.win.sync(ctx, &mut self.cfg, &mut self.save_at);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(if self.settings_open {
            90
        } else {
            220
        }));
    }
}

/// 累计行标签：历史快照要标出来，否则用户会以为数字还在涨
fn total_label(stale: bool, turns: usize) -> String {
    if stale {
        if turns > 0 {
            format!("累计 · {} 轮 · 缓存", turns)
        } else {
            "累计 · 缓存".to_string()
        }
    } else if turns > 0 {
        format!("累计 · {} 轮", turns)
    } else {
        "累计".to_string()
    }
}
