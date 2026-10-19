//! 独立的设置窗口（系统标题栏 + 自己的页面），改动实时作用到 bar 上。
//!
//! 用 egui 的 deferred viewport 开一个真正的独立 OS 窗口，而不是在 bar 内部画浮层。

use crate::config::{Config, LayoutMode, ThemePref};
use crate::metric::Metric;
use crate::ui::{
    app_icon, card_bg, check, draw_layout_thumb, fam, mix, on_accent, page_bg, palette, switch,
    text_at, text_w, Palette, FONT_R, FONT_SB,
};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind,
};
use std::sync::{Arc, Mutex};

pub const WIN_W: f32 = 420.0;
pub const WIN_H: f32 = 700.0;

const PAD: f32 = 20.0;
const ROW_H: f32 = 36.0;
const SEG_H: f32 = 34.0;
const BTN_H: f32 = 34.0;
/// 版本卡片的高度
const VER_H: f32 = 52.0;
const VER_BTN_W: f32 = 84.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Commit,
    Cancel,
}

/// 设置窗口里"版本信息"那一块要用户做的事
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UpdateCmd {
    Check,
    Download,
    Restart,
    /// 这个平台不做应用内更新（macOS）：直接打开发布页
    OpenRelease,
}

/// 版本信息那一行的状态（由 App 从 update::Status 翻出来）
#[derive(Clone, Debug, PartialEq)]
pub struct UpInfo {
    /// 第一行小字：版本号 + 打包时间
    pub line1: String,
    /// 第二行状态
    pub status: String,
    pub button: &'static str,
    /// 按钮主色（有活可干）还是普通色（只是"检测"）
    pub primary: bool,
    /// 检测中/下载中：按钮点了也没用
    pub busy: bool,
    pub progress: Option<f32>,
    pub cmd: UpdateCmd,
}

impl Default for UpInfo {
    fn default() -> Self {
        Self {
            line1: String::new(),
            status: "还没检测过".into(),
            button: "检测更新",
            primary: false,
            busy: false,
            progress: None,
            cmd: UpdateCmd::Check,
        }
    }
}

/// 设置窗口相对 bar 放在哪一侧（永远不盖住 bar）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Above,
    Below,
    Right,
    Left,
}

/// 设置窗口与 bar 之间共享的草稿状态
pub struct Draft {
    pub open: bool,
    /// 用户正在编辑的配置（实时作用到 bar，但只有点"确定"才写盘）
    pub cfg: Config,
    /// 打开时的快照，用于"取消"
    pub orig: Config,
    pub result: Option<Outcome>,
    pub dark: bool,
    pub first: bool,
    pub focus: bool,
    pub win_pos: Pos2,
    /// bar 当前矩形（逻辑坐标），用来把设置窗口摆在它外面
    pub bar_rect: Rect,
    /// bar 所在那块屏幕的矩形（逻辑坐标；副屏左上角不是 0,0）
    pub mon_rect: Rect,
    pub place: Place,
    /// 首帧拿到真实窗口尺寸后再精确贴边一次
    pub fit: bool,
    /// 版本信息那一块（每帧由 App 刷新）
    pub up: UpInfo,
    /// 用户在设置里点的更新动作，App 每帧取走
    pub up_cmd: Option<UpdateCmd>,
    /// macOS 没给「自动化」授权：查不到前台应用，跟随显示/隐藏其实不会生效
    pub no_automation: bool,
    /// 正在拖动的指标：(指标, 指针相对行顶的偏移)
    drag: Option<(Metric, f32)>,
}

impl Draft {
    pub fn new(cfg: &Config) -> Self {
        Self {
            open: false,
            cfg: cfg.clone(),
            orig: cfg.clone(),
            result: None,
            dark: true,
            first: true,
            focus: true,
            win_pos: pos2(0.0, 0.0),
            bar_rect: Rect::NOTHING,
            mon_rect: Rect::NOTHING,
            place: Place::Above,
            fit: false,
            up: UpInfo::default(),
            up_cmd: None,
            no_automation: false,
            drag: None,
        }
    }

    /// 打开/关闭设置时清掉未完成的拖动
    pub fn reset_drag(&mut self) {
        self.drag = None;
    }
}

/// 设置窗口的视口 id（bar 那边需要给它发重绘/位置命令）
pub fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("tokenbar-settings")
}

pub fn show(ctx: &egui::Context, shared: Arc<Mutex<Draft>>) {
    let (open, first) = {
        let Ok(d) = shared.lock() else {
            return;
        };
        (d.open, d.first)
    };
    if !open {
        return;
    }

    let mut vb = egui::ViewportBuilder::default()
        .with_title("Tokenbar 设置")
        .with_inner_size([WIN_W, WIN_H])
        .with_min_inner_size([WIN_W, WIN_H])
        .with_max_inner_size([WIN_W, WIN_H])
        .with_resizable(false)
        .with_transparent(false)
        .with_icon(Arc::new(app_icon()))
        .with_always_on_top();

    if first {
        if let Ok(mut d) = shared.lock() {
            vb = vb.with_position(d.win_pos);
            d.first = false;
        }
    }

    ctx.show_viewport_deferred(viewport_id(), vb, move |ctx, _class| {
        let Ok(mut d) = shared.lock() else {
            return;
        };
        if !d.open {
            return;
        }
        // 点系统标题栏的 × 等同于取消
        if ctx.input(|i| i.viewport().close_requested()) {
            if d.result.is_none() {
                d.result = Some(Outcome::Cancel);
            }
            return;
        }
        if d.focus {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            d.focus = false;
        }
        fit_around_bar(ctx, &mut d);
        // 这里不能再用 ctx.root_ui（外层 run_ui 已经借走了根 Ui），
        // 直接给这个视口造一个铺满窗口的 Ui。
        let rect = ctx.viewport_rect();
        let mut ui = egui::Ui::new(
            ctx.clone(),
            ctx.viewport_id().root_ui_id(),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(rect),
        );
        draw(&mut ui, &mut d);
    });
}

/// 挑一个能完整放下设置窗口、又不压住 bar 的方向
fn best_place(bar: Rect, outer: egui::Vec2, mon: Rect) -> Place {
    let gap = 12.0;
    if bar.top() - gap - outer.y >= mon.top() + 8.0 {
        Place::Above
    } else if bar.bottom() + gap + outer.y <= mon.bottom() - 8.0 {
        Place::Below
    } else if bar.right() + gap + outer.x <= mon.right() - 8.0 {
        Place::Right
    } else if bar.left() - gap - outer.x >= mon.left() + 8.0 {
        Place::Left
    } else {
        Place::Above
    }
}

/// 用真实窗口尺寸把设置窗口贴到 bar 外面（标题栏高度、DPI 都不用猜）
fn fit_around_bar(ctx: &egui::Context, d: &mut Draft) {
    if !d.fit {
        return;
    }
    let Some(outer) = ctx.input(|i| i.viewport().outer_rect) else {
        return;
    };
    if outer.width() < 100.0 {
        return;
    }
    // 用 App 每帧算好的屏幕矩形；拿不到就按"屏幕左上角是 0,0"退回去
    let mon = if d.mon_rect.is_positive() {
        d.mon_rect
    } else {
        let size = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(egui::vec2(1920.0, 1080.0));
        Rect::from_min_size(Pos2::ZERO, size)
    };
    let bar = d.bar_rect;
    let gap = 12.0;
    let place = best_place(bar, outer.size(), mon);
    d.place = place;
    let mut p = match place {
        Place::Above => pos2(
            bar.center().x - outer.width() * 0.5,
            bar.top() - gap - outer.height(),
        ),
        Place::Below => pos2(bar.center().x - outer.width() * 0.5, bar.bottom() + gap),
        Place::Right => pos2(bar.right() + gap, bar.center().y - outer.height() * 0.5),
        Place::Left => pos2(
            bar.left() - gap - outer.width(),
            bar.center().y - outer.height() * 0.5,
        ),
    };
    p.x = p
        .x
        .clamp(mon.left() + 8.0, (mon.right() - outer.width() - 8.0).max(mon.left() + 8.0));
    p.y = p
        .y
        .clamp(mon.top() + 8.0, (mon.bottom() - outer.height() - 8.0).max(mon.top() + 8.0));
    if (p - outer.min).length() > 0.5 {
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(p));
    }
    d.fit = false;
}

/// 整页自上而下：外观 → 布局 → 行为 → 指标 → 底部按钮
fn draw(ui: &mut egui::Ui, d: &mut Draft) {
    let pal = palette(d.dark);
    let area = ui.max_rect();
    ui.painter()
        .rect_filled(area, CornerRadius::same(0), page_bg(&pal));
    let left = area.left() + PAD;
    let right = area.right() - PAD;
    let w = right - left;
    let mut y = area.top() + 18.0;

    section_label(ui, &pal, left, y, "外观");
    y += 20.0;
    y = segmented(ui, d, &pal, left, y, w) + 22.0;

    section_label(ui, &pal, left, y, "布局");
    y += 20.0;
    y = layout_cards(ui, d, &pal, left, y, w) + 22.0;

    section_label(ui, &pal, left, y, "行为");
    y += 18.0;
    y = follow_row(ui, d, &pal, left, y, w) + 14.0;

    ui.painter().line_segment(
        [pos2(left, y), pos2(right, y)],
        Stroke::new(1.0, pal.divider),
    );
    y += 16.0;

    y = metric_header(ui, d, &pal, left, right, y);
    metric_rows(ui, d, &pal, left, y, w);
    y += ROW_H * Metric::ALL.len() as f32;

    // ---- 版本信息
    ui.painter().line_segment(
        [pos2(left, y + 13.0), pos2(right, y + 13.0)],
        Stroke::new(1.0, pal.divider),
    );
    y += 32.0;
    section_label(ui, &pal, left, y - 14.0, "版本");
    version_card(ui, d, &pal, left, y, w);
    y += VER_H;

    footer(ui, d, &pal, area, left, right, y);
}

/// 版本号 + 打包时间 + 检测更新（发现新版时按钮变成"更新"）
fn version_card(ui: &mut egui::Ui, d: &mut Draft, pal: &Palette, left: f32, y: f32, w: f32) {
    let r = Rect::from_min_size(pos2(left, y), vec2(w, VER_H));
    ui.painter()
        .rect_filled(r, CornerRadius::same(12), card_bg(pal));
    ui.painter().rect_stroke(
        r,
        CornerRadius::same(12),
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );

    let btn = Rect::from_min_size(
        pos2(r.right() - 14.0 - VER_BTN_W, r.top() + (VER_H - 28.0) * 0.5),
        vec2(VER_BTN_W, 28.0),
    );
    let text_max = btn.left() - 10.0 - (r.left() + 14.0);

    text_at(
        ui.painter(),
        pos2(r.left() + 14.0, r.top() + 16.0),
        Align2::LEFT_CENTER,
        &clip(ui.ctx(), &d.up.line1, text_max, 10.0, FONT_R),
        10.0,
        FONT_R,
        pal.faint,
    );
    let tone = if d.up.progress.is_some() || d.up.primary {
        pal.accent
    } else {
        pal.text
    };
    text_at(
        ui.painter(),
        pos2(r.left() + 14.0, r.top() + 35.0),
        Align2::LEFT_CENTER,
        &clip(ui.ctx(), &d.up.status, text_max, 11.5, FONT_SB),
        11.5,
        FONT_SB,
        tone,
    );

    // 下载进度：卡片底边一条细线
    if let Some(t) = d.up.progress {
        let track = Rect::from_min_size(
            pos2(r.left() + 12.0, r.bottom() - 6.0),
            vec2(r.width() - 24.0, 3.0),
        );
        ui.painter()
            .rect_filled(track, CornerRadius::same(2), pal.track);
        if t > 0.01 {
            let fill = Rect::from_min_size(track.min, vec2(track.width() * t, 3.0));
            ui.painter()
                .rect_filled(fill, CornerRadius::same(2), pal.accent);
        }
    }

    let resp = clickable(ui, btn, sid("update"));
    let hot = resp.hovered() && !d.up.busy;
    let (bg, fg) = if d.up.primary {
        (
            if hot {
                mix(pal.accent, Color32::WHITE, 0.12)
            } else {
                pal.accent
            },
            on_accent(pal),
        )
    } else {
        (
            if hot { pal.hover } else { pal.surface },
            if d.up.busy { pal.faint } else { pal.text },
        )
    };
    ui.painter().rect_filled(btn, CornerRadius::same(8), bg);
    text_at(
        ui.painter(),
        btn.center(),
        Align2::CENTER_CENTER,
        d.up.button,
        11.0,
        FONT_R,
        fg,
    );
    if resp.clicked() && !d.up.busy {
        d.up_cmd = Some(d.up.cmd);
    }
}

/// 按宽度裁字，尾巴补省略号
fn clip(ctx: &egui::Context, text: &str, max_w: f32, size: f32, font: &str) -> String {
    if text.is_empty() || max_w <= 8.0 {
        return String::new();
    }
    if text_w(ctx, text, size, font) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 1 {
        let s: String = chars[..n].iter().collect();
        if text_w(ctx, &format!("{s}…"), size, font) <= max_w {
            return format!("{s}…");
        }
        n -= 1;
    }
    "…".to_string()
}

fn section_label(ui: &mut egui::Ui, pal: &Palette, x: f32, y: f32, text: &str) {
    ui.painter().text(
        pos2(x, y),
        Align2::LEFT_CENTER,
        text,
        FontId::new(10.5, fam(FONT_R)),
        pal.faint,
    );
}

fn clickable(ui: &mut egui::Ui, rect: Rect, id: egui::Id) -> egui::Response {
    ui.interact(rect, id, Sense::click())
}

#[allow(deprecated)]
fn sid(parts: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
    egui::Id::new(("tokenbar-settings", parts))
}

// ---------------------------------------------------------------- 主题分段

fn segmented(ui: &mut egui::Ui, d: &mut Draft, pal: &Palette, left: f32, y: f32, w: f32) -> f32 {
    let track = Rect::from_min_size(pos2(left, y), vec2(w, SEG_H));
    ui.painter()
        .rect_filled(track, CornerRadius::same(10), pal.track);
    let gap = 3.0;
    let cell_w = (w - gap * 4.0) / 3.0;
    for (i, t) in [ThemePref::System, ThemePref::Dark, ThemePref::Light]
        .iter()
        .enumerate()
    {
        let r = Rect::from_min_size(
            pos2(left + gap + (cell_w + gap) * i as f32, y + gap),
            vec2(cell_w, SEG_H - gap * 2.0),
        );
        let on = d.cfg.theme == *t;
        let resp = clickable(ui, r, sid(("theme", i)));
        if on {
            // 选中项：白色卡片浮起 + 淡影，不用描边
            for (e, a) in [(0.8f32, 0.10f32), (2.0, 0.05)] {
                ui.painter().rect_stroke(
                    r.expand(e),
                    CornerRadius::same(9),
                    Stroke::new(1.6, pal.shadow.gamma_multiply(a)),
                    StrokeKind::Outside,
                );
            }
            ui.painter()
                .rect_filled(r, CornerRadius::same(8), card_bg(pal));
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(r, CornerRadius::same(8), pal.hover);
        }
        text_at(
            ui.painter(),
            r.center(),
            Align2::CENTER_CENTER,
            t.label(),
            11.5,
            FONT_R,
            if on { pal.accent } else { pal.muted },
        );
        if resp.clicked() {
            d.cfg.theme = *t;
        }
    }
    y + SEG_H
}

// ---------------------------------------------------------------- 布局卡片

fn layout_cards(ui: &mut egui::Ui, d: &mut Draft, pal: &Palette, left: f32, y: f32, w: f32) -> f32 {
    let card_w = (w - 10.0) / 2.0;
    let card_h = 70.0;
    for (i, mode) in [LayoutMode::Horizontal, LayoutMode::Vertical]
        .iter()
        .enumerate()
    {
        let r = Rect::from_min_size(
            pos2(left + (card_w + 10.0) * i as f32, y),
            vec2(card_w, card_h),
        );
        let on = d.cfg.layout == *mode;
        let resp = clickable(ui, r, sid(("layout", i)));
        ui.painter().rect_filled(
            r,
            CornerRadius::same(10),
            if on {
                pal.accent.gamma_multiply(0.06)
            } else {
                card_bg(pal)
            },
        );
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(10),
            Stroke::new(
                if on { 1.4 } else { 1.0 },
                if on {
                    pal.accent.gamma_multiply(0.8)
                } else {
                    pal.border
                },
            ),
            StrokeKind::Inside,
        );
        if resp.hovered() && !on {
            ui.painter()
                .rect_filled(r, CornerRadius::same(10), pal.hover);
        }
        draw_layout_thumb(ui.painter(), r, *mode == LayoutMode::Vertical, pal, on);
        text_at(
            ui.painter(),
            pos2(r.center().x, r.bottom() - 12.0),
            Align2::CENTER_CENTER,
            mode.label(),
            10.5,
            FONT_R,
            if on { pal.accent } else { pal.muted },
        );
        if resp.clicked() {
            d.cfg.layout = *mode;
        }
    }
    y + card_h
}

// ---------------------------------------------------------------- 跟随开关

fn follow_row(ui: &mut egui::Ui, d: &mut Draft, pal: &Palette, left: f32, y: f32, w: f32) -> f32 {
    let row = Rect::from_min_size(pos2(left, y), vec2(w, 46.0));
    let resp = clickable(ui, row, sid("follow"));
    // 卡片式：开关说明收进一张白卡片里
    ui.painter()
        .rect_filled(row, CornerRadius::same(12), card_bg(pal));
    ui.painter().rect_stroke(
        row,
        CornerRadius::same(12),
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );
    if resp.hovered() {
        ui.painter()
            .rect_filled(row, CornerRadius::same(12), pal.hover);
    }
    ui.painter().text(
        pos2(left + 14.0, row.top() + 14.0),
        Align2::LEFT_CENTER,
        "跟随 WorkBuddy 显示",
        FontId::new(12.0, fam(FONT_SB)),
        pal.text,
    );
    let hint = if d.no_automation {
        "未授权「自动化」：暂时不会自动隐身"
    } else {
        "切到其它应用时自动隐身，回来再出现"
    };
    ui.painter().text(
        pos2(left + 14.0, row.top() + 32.0),
        Align2::LEFT_CENTER,
        hint,
        FontId::new(10.0, fam(FONT_R)),
        pal.faint,
    );
    let sw = Rect::from_min_size(pos2(row.right() - 14.0 - 40.0, row.top() + 13.0), vec2(40.0, 21.0));
    switch(ui.painter(), sw, d.cfg.follow, pal);
    if resp.clicked() {
        d.cfg.follow = !d.cfg.follow;
    }
    y + 46.0
}

// ---------------------------------------------------------------- 指标列表

/// "指标 4 / 6" + 右侧"恢复默认"
fn metric_header(
    ui: &mut egui::Ui,
    d: &mut Draft,
    pal: &Palette,
    left: f32,
    right: f32,
    y: f32,
) -> f32 {
    section_label(ui, pal, left, y, "指标");
    let shown = d.cfg.visible_metrics().len();
    // 数量做成小徽章
    let cnt = format!("{} / {}", shown, Metric::ALL.len());
    let cw = text_w(ui.ctx(), &cnt, 10.0, FONT_SB);
    let badge = Rect::from_min_size(pos2(left + 30.0, y - 9.0), vec2(cw + 14.0, 18.0));
    ui.painter().rect_filled(
        badge,
        CornerRadius::same(9),
        pal.accent.gamma_multiply(0.10),
    );
    text_at(
        ui.painter(),
        badge.center(),
        Align2::CENTER_CENTER,
        &cnt,
        10.0,
        FONT_SB,
        pal.accent,
    );

    let reset = Rect::from_min_size(pos2(right - 64.0, y - 11.0), vec2(64.0, 22.0));
    let resp = clickable(ui, reset, sid("reset"));
    if resp.hovered() {
        ui.painter()
            .rect_filled(reset, CornerRadius::same(6), pal.hover);
    }
    text_at(
        ui.painter(),
        reset.center(),
        Align2::CENTER_CENTER,
        "恢复默认",
        10.5,
        FONT_R,
        if resp.hovered() {
            pal.accent2
        } else {
            pal.muted
        },
    );
    if resp.clicked() {
        d.cfg.order = Config::default_order();
        for m in Metric::ALL {
            d.cfg.set_enabled(m, true);
        }
    }
    y + 18.0
}

fn metric_rows(ui: &mut egui::Ui, d: &mut Draft, pal: &Palette, left: f32, y: f32, w: f32) {
    let order = d.cfg.order.clone();
    let mut toggle: Option<Metric> = None;
    let mut drag_started: Option<(Metric, f32)> = None;

    for (i, key) in order.iter().enumerate() {
        let key = *key;
        let row = Rect::from_min_size(pos2(left, y + i as f32 * ROW_H), vec2(w, ROW_H - 4.0));
        let dragging = d.drag.map(|(k, _)| k == key).unwrap_or(false);
        let resp = ui.interact(row, sid(("metric", key.key())), Sense::click_and_drag());
        if resp.drag_started() {
            if let Some(q) = resp.interact_pointer_pos() {
                drag_started = Some((key, q.y - row.top()));
            }
        }

        // 行与行之间的细分隔线（拖拽中的行不画，让它浮起来）
        if i + 1 < order.len() && !dragging {
            let ly = row.bottom() + 2.0;
            ui.painter().line_segment(
                [pos2(left + 4.0, ly), pos2(left + w - 4.0, ly)],
                Stroke::new(1.0, pal.divider.gamma_multiply(0.7)),
            );
        }

        let hot = resp.hovered() || dragging;
        if hot {
            ui.painter()
                .rect_filled(row, CornerRadius::same(8), pal.hover);
        }
        if dragging {
            ui.painter().rect_stroke(
                row,
                CornerRadius::same(8),
                Stroke::new(1.2, pal.accent.gamma_multiply(0.7)),
                StrokeKind::Inside,
            );
        }

        // 拖动把手
        let grip = if hot { pal.muted } else { pal.faint };
        for k in 0..3 {
            let gy = row.center().y - 4.5 + k as f32 * 4.5;
            ui.painter().line_segment(
                [pos2(row.left() + 9.0, gy), pos2(row.left() + 17.0, gy)],
                Stroke::new(1.3, grip),
            );
        }

        let enabled = d.cfg.is_enabled(key);
        text_at(
            ui.painter(),
            pos2(row.left() + 30.0, row.center().y),
            Align2::LEFT_CENTER,
            key.full_label(),
            12.0,
            FONT_R,
            if enabled { pal.text } else { pal.faint },
        );

        // 圆形勾选：选中实心带勾，未选空心圈
        let cc = pos2(row.right() - 14.0, row.center().y);
        if enabled {
            ui.painter().circle_filled(cc, 10.0, pal.accent);
            check(ui.painter(), cc, on_accent(pal));
        } else {
            ui.painter().circle_stroke(cc, 9.5, Stroke::new(1.5, pal.border));
        }
        // 点右端勾选框 = 切换显示（点其它地方不响应，避免误触）
        let cbox = Rect::from_center_size(cc, vec2(19.0, 19.0));
        if resp.clicked() {
            if let Some(q) = resp.interact_pointer_pos() {
                if cbox.expand(6.0).contains(q) {
                    toggle = Some(key);
                }
            }
        }
    }

    if let Some((key, off)) = drag_started {
        d.drag = Some((key, off));
    }
    // 拖动中：按指针位置换位
    if let Some((key, _)) = d.drag {
        let down = ui.input(|i| i.pointer.primary_down());
        if down {
            if let Some(q) = ui.input(|i| i.pointer.interact_pos()) {
                let idx =
                    (((q.y - y) / ROW_H).floor() as i32).clamp(0, order.len() as i32 - 1) as usize;
                if let Some(cur) = d.cfg.order.iter().position(|k| *k == key) {
                    if cur != idx {
                        let k = d.cfg.order.remove(cur);
                        d.cfg.order.insert(idx, k);
                    }
                }
            }
        } else {
            d.drag = None;
        }
    }
    if let Some(key) = toggle {
        // 至少留一项，否则用户会看到空 bar
        if !(d.cfg.is_enabled(key) && d.cfg.visible_metrics().len() <= 1) {
            let on = d.cfg.is_enabled(key);
            d.cfg.set_enabled(key, !on);
        }
    }
}

// ---------------------------------------------------------------- 底部

fn footer(
    ui: &mut egui::Ui,
    d: &mut Draft,
    pal: &Palette,
    area: Rect,
    left: f32,
    right: f32,
    list_bottom: f32,
) {
    let by = (area.bottom() - 18.0 - BTN_H).min(list_bottom + 18.0);
    // 底部通栏：卡片底 + 顶部分隔线，把按钮区和列表隔开
    let band = Rect::from_min_max(pos2(area.left(), by - 12.0), area.max);
    ui.painter()
        .rect_filled(band, CornerRadius::same(0), card_bg(pal));
    ui.painter().line_segment(
        [pos2(area.left(), by - 12.0), pos2(area.right(), by - 12.0)],
        Stroke::new(1.0, pal.divider),
    );
    text_at(
        ui.painter(),
        pos2(left, by + BTN_H * 0.5),
        Align2::LEFT_CENTER,
        &format!("改动会立刻显示在 Tokenbar 上 · {}", crate::ui::version()),
        9.5,
        FONT_R,
        pal.faint,
    );

    let ok_r = Rect::from_min_size(pos2(right - 92.0, by), vec2(92.0, BTN_H));
    let cancel_r = Rect::from_min_size(pos2(right - 92.0 - 8.0 - 78.0, by), vec2(78.0, BTN_H));

    let cancel = clickable(ui, cancel_r, sid("cancel"));
    ui.painter().rect_filled(
        cancel_r,
        CornerRadius::same(9),
        if cancel.hovered() {
            pal.hover
        } else {
            pal.surface
        },
    );
    text_at(
        ui.painter(),
        cancel_r.center(),
        Align2::CENTER_CENTER,
        "取消",
        12.0,
        FONT_R,
        pal.text,
    );
    if cancel.clicked() {
        d.result = Some(Outcome::Cancel);
    }

    let ok = clickable(ui, ok_r, sid("ok"));
    let ok_bg = if ok.hovered() {
        mix(pal.accent, Color32::WHITE, 0.12)
    } else {
        pal.accent
    };
    // 主按钮带一点投影，主次更分明
    for (e, a) in [(1.0f32, 0.35f32), (3.0, 0.16)] {
        ui.painter().rect_stroke(
            ok_r.expand(e),
            CornerRadius::same(11),
            Stroke::new(2.0, pal.accent.gamma_multiply(a)),
            StrokeKind::Outside,
        );
    }
    ui.painter().rect_filled(ok_r, CornerRadius::same(9), ok_bg);
    text_at(
        ui.painter(),
        ok_r.center(),
        Align2::CENTER_CENTER,
        "确定",
        12.0,
        FONT_R,
        on_accent(pal),
    );
    if ok.clicked() {
        d.result = Some(Outcome::Commit);
    }
}
