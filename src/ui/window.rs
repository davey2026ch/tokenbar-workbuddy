//! bar 窗口本身的位置与尺寸。
//!
//! 几条踩过的坑都收在这里：
//! - 隐藏用"挪到屏幕外"，不用 ShowWindow（Windows 下恢复后会收不到鼠标事件）
//! - 定位下发的是**外层**坐标，隐藏时也要跟着挪
//! - 布局切换导致尺寸变化时，靠近哪边就以哪边为锚

use crate::config::Config;
use eframe::egui::{self, pos2, vec2, Pos2, Rect, Vec2};
use std::time::Instant;

/// 屏幕外的"藏起来"坐标
pub const OFFSCREEN: f32 = -30000.0;
/// 默认落位时离屏幕底部留出的距离（逻辑点）
pub const BOTTOM_MARGIN: f32 = 118.0;
/// 拖动结束后多久落盘
const SAVE_AFTER_MS: u128 = 300;

#[derive(Default)]
pub struct WindowState {
    origin: Option<Pos2>,
    last_outer: Option<Pos2>,
    pending_move: Option<Pos2>,
    cur_size: Vec2,
    last_view: Vec2,
}

impl WindowState {
    pub fn origin(&self) -> Option<Pos2> {
        self.origin
    }

    pub fn view_size(&self) -> Vec2 {
        self.last_view
    }

    /// 当前 bar 占的矩形（还不知道尺寸时给个默认横排大小）
    pub fn rect(&self) -> Rect {
        let size = if self.last_view.x > 0.0 {
            self.last_view
        } else {
            vec2(440.0, 70.0)
        };
        Rect::from_min_size(self.origin.unwrap_or_else(|| pos2(0.0, 0.0)), size)
    }

    /// 这一帧 bar 的左上角：首次落位、尺寸变化贴边、夹进屏幕。
    ///
    /// `mon` 是 bar 现在所在那块屏幕的工作区（逻辑点，左上角不一定是 0,0——副屏就是这样）；
    /// `allow_clamp` 为 false 时不做"夹回屏幕内"，用于刚被系统级移动到目标屏幕的那几帧。
    pub fn resolve_origin(
        &mut self,
        cfg: &mut Config,
        mon: Rect,
        view: Vec2,
        allow_clamp: bool,
    ) -> Pos2 {
        if self.origin.is_none() {
            let pos = match (cfg.x, cfg.y) {
                (Some(x), Some(y))
                    if x > mon.left() - view.x + 40.0
                        && x < mon.right() - 40.0
                        && y >= mon.top()
                        && y < mon.bottom() - 24.0 =>
                {
                    pos2(x, y)
                }
                _ => pos2(
                    (mon.left() + (mon.width() - view.x) * 0.5).round(),
                    (mon.bottom() - view.y - BOTTOM_MARGIN).max(mon.top() + 24.0).round(),
                ),
            };
            self.origin = Some(pos);
            cfg.set_position(pos.x, pos.y);
            cfg.save();
        }

        let mut origin = self.origin.unwrap_or_else(|| pos2(0.0, 0.0));
        let mut moved = false;
        // 高度变了：贴底的话往上长，否则顶部不动
        if self.last_view.x > 0.0 && (view.y - self.last_view.y).abs() > 0.5 {
            let bottom_margin = mon.bottom() - (origin.y + self.last_view.y);
            if bottom_margin < origin.y - mon.top() {
                origin.y -= view.y - self.last_view.y;
                moved = true;
            }
        }
        // 宽度变了：保持水平居中
        if self.last_view.x > 0.0 && (view.x - self.last_view.x).abs() > 0.5 {
            origin.x += (self.last_view.x - view.x) * 0.5;
            moved = true;
        }
        self.last_view = view;
        if allow_clamp {
            let max_x = (mon.right() - view.x).max(mon.left());
            let max_y = (mon.bottom() - view.y).max(mon.top());
            if origin.x < mon.left() {
                origin.x = mon.left();
                moved = true;
            }
            if origin.x > max_x {
                origin.x = max_x;
                moved = true;
            }
            if origin.y < mon.top() {
                origin.y = mon.top();
                moved = true;
            }
            if origin.y > max_y {
                origin.y = max_y;
                moved = true;
            }
        }
        if moved {
            self.origin = Some(origin);
            cfg.set_position(origin.x, origin.y);
            cfg.save();
        }
        origin
    }

    /// 系统层把窗口挪走之后同步内部状态，别把它当成"用户拖动"
    pub fn adopt(&mut self, origin: Pos2, view: Vec2) {
        self.origin = Some(origin);
        self.pending_move = None;
        self.last_outer = Some(origin);
        self.cur_size = view;
        self.last_view = view;
    }

    /// 尺寸变化时下发给系统；隐藏状态下要落到屏幕外
    pub fn apply_size(&mut self, ctx: &egui::Context, view: Vec2, origin: Pos2, visible: bool) {
        if (self.cur_size.x - view.x).abs() <= 0.5 && (self.cur_size.y - view.y).abs() <= 0.5 {
            return;
        }
        let target = if visible {
            origin
        } else {
            pos2(OFFSCREEN, OFFSCREEN)
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(view));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target));
        self.cur_size = view;
        self.pending_move = Some(target);
        self.last_outer = Some(target);
    }

    /// 跟随 WorkBuddy 显隐：整窗移出 / 移回屏幕（不用 ShowWindow）
    pub fn move_offscreen(&mut self, ctx: &egui::Context, hidden: bool) {
        let Some(origin) = self.origin else {
            return;
        };
        let target = if hidden {
            pos2(OFFSCREEN, OFFSCREEN)
        } else {
            origin
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target));
        self.pending_move = Some(target);
        self.last_outer = Some(target);
    }

    /// 拖动后跟着窗口走，松手 300ms 再落盘
    pub fn sync(&mut self, ctx: &egui::Context, cfg: &mut Config, save_at: &mut Option<Instant>) {
        let Some(outer) = ctx.input(|i| i.viewport().outer_rect) else {
            return;
        };
        let top_left = outer.min;
        if let Some(pending) = self.pending_move {
            if (top_left - pending).length() < 2.5 {
                self.pending_move = None;
                self.last_outer = Some(top_left);
            }
            return;
        }
        match self.last_outer {
            Some(last) if (top_left - last).length() > 0.5 => {
                self.last_outer = Some(top_left);
                self.origin = Some(top_left);
                cfg.set_position(top_left.x, top_left.y);
                *save_at = Some(Instant::now());
            }
            None => self.last_outer = Some(top_left),
            _ => {}
        }
        if let Some(at) = *save_at {
            if at.elapsed().as_millis() > SAVE_AFTER_MS {
                cfg.save();
                *save_at = None;
            }
        }
    }
}
