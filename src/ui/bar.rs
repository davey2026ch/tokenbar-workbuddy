//! 指标卡片：横排（仪表盘三层次）与竖排（两列）两种版式的测量与绘制。
//!
//! 横排一列 = 顶部小标签 + 本轮大数字（可带命中率小后缀）+ 底部灰色累计值，
//! 「本轮 / 累计 · N 轮」固定在左侧只出现一次；右侧是独立的悬浮按钮区。

use super::chips::{self, Chip};
use super::icons::{close_icon, sliders_icon};
use super::text::{at, label, number, text_w, FONT_R, FONT_SB};
use super::theme::Palette;
use eframe::egui::{
    self, pos2, vec2, Align2, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind,
};

pub const BAR_H: f32 = 76.0;
const PAD_L: f32 = 14.0;
/// 右侧悬浮按钮区的宽度（不悬浮时就是一点留白）
const BTN_ZONE: f32 = 36.0;
const BTN_SIZE: f32 = 24.0;
const BTN_GAP: f32 = 7.0;
/// 指标列左右各留的内边距
const COL_PAD: f32 = 20.0;

/// 横排三层文字的垂直中心（相对卡片顶）
const LABEL_CY: f32 = 14.5;
const VALUE_CY: f32 = 38.5;
const SUB_CY: f32 = 60.5;

const VALUE_H: f32 = 15.5;
const SUB_H: f32 = 10.5;

/// 两行指标 + 左侧行标签
pub struct Rows<'a> {
    pub turn: &'a [Chip],
    pub total: &'a [Chip],
    pub turn_label: &'a str,
    pub total_label: &'a str,
}

/// 一帧要画的东西
pub struct View<'a> {
    pub rows: Rows<'a>,
    pub pal: &'a Palette,
    pub time: f32,
    /// 正在生成（流式输出中）：圆点会呼吸
    pub live: bool,
    /// 任务在跑但还没到流式阶段（跑工具 / 思考）：用点缀色，但不呼吸
    pub active: bool,
}

const EMPTY_TITLE: &str = "当前任务还没有 Token 记录";
const EMPTY_HINT: &str = "在 WorkBuddy 里开始对话后，这里会自动出现指标";

/// 空态（新任务页 / 新会话还没开跑）：两种版式共用一张窄卡，别摆一个大空盒子
pub fn measure_empty(ctx: &egui::Context) -> egui::Vec2 {
    let l1 = text_w(ctx, "Tokenbar", 12.5, FONT_SB) + 10.0 + text_w(ctx, EMPTY_TITLE, 12.0, FONT_R);
    let l2 = text_w(ctx, EMPTY_HINT, 10.0, FONT_R);
    let w = PAD_L + 13.0 + l1.max(l2) + 16.0 + BTN_ZONE;
    vec2(w.max(300.0), BAR_H)
}

pub fn draw_empty(ctx: &egui::Context, p: &Painter, body: Rect, pal: &Palette, time: f32) {
    let top = body.top();
    let cy1 = top + 31.0;
    let pulse = 0.72 + 0.28 * (time * 4.0).sin();
    p.circle_filled(pos2(PAD_L + 3.0, cy1), 2.6, pal.faint.gamma_multiply(pulse));
    number(p, PAD_L + 13.0, cy1, "Tokenbar", 12.5, pal.text);
    let bw = text_w(ctx, "Tokenbar", 12.5, FONT_SB);
    at(
        p,
        pos2(PAD_L + 13.0 + bw + 10.0, cy1),
        Align2::LEFT_CENTER,
        EMPTY_TITLE,
        12.0,
        FONT_R,
        pal.muted,
    );
    label(p, PAD_L + 1.0, top + 55.0, EMPTY_HINT, 10.0, pal.faint);
}

// ---------------------------------------------------------------- 卡片底

pub fn card(p: &Painter, rect: Rect, pal: &Palette, radius: u8) {
    for i in 0..4 {
        let e = 1.2 + i as f32 * 2.0;
        let a = if pal.dark {
            0.20 - i as f32 * 0.045
        } else {
            0.13 - i as f32 * 0.03
        };
        p.rect_stroke(
            rect.expand(e),
            CornerRadius::same(radius + 2),
            Stroke::new(2.6, pal.shadow.gamma_multiply(a.max(0.02))),
            StrokeKind::Outside,
        );
    }
    p.rect_filled(rect, CornerRadius::same(radius), pal.bg);
    p.rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );
}

// ---------------------------------------------------------------- 悬浮按钮

/// 卡片右边缘的两个竖排按钮：返回 (设置, 关闭)，设置在下、关闭在上
pub fn hover_buttons(body: Rect) -> (Rect, Rect) {
    let x = body.right() - (BTN_ZONE * 0.5);
    let cy = body.center().y;
    let off = (BTN_SIZE + BTN_GAP) * 0.5;
    (
        Rect::from_center_size(pos2(x, cy + off), vec2(BTN_SIZE, BTN_SIZE)),
        Rect::from_center_size(pos2(x, cy - off), vec2(BTN_SIZE, BTN_SIZE)),
    )
}

pub fn draw_buttons(p: &Painter, pal: &Palette, body: Rect, pointer: Option<Pos2>, alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    if a < 0.02 {
        return;
    }
    // 按钮区与指标列之间的分隔线，跟按钮一起淡入
    let dx = body.right() - BTN_ZONE;
    p.line_segment(
        [pos2(dx, body.top() + 20.0), pos2(dx, body.bottom() - 20.0)],
        Stroke::new(1.0, pal.divider.gamma_multiply(a)),
    );
    let (gear, close) = hover_buttons(body);
    for (r, setup) in [(gear, true), (close, false)] {
        let hot = pointer.map(|q| r.contains(q)).unwrap_or(false);
        let bg = if hot { pal.hover } else { pal.surface };
        p.rect_filled(
            r,
            CornerRadius::same(8),
            bg.gamma_multiply(a * if hot { 1.4 } else { 0.9 }),
        );
        if hot {
            p.rect_stroke(
                r,
                CornerRadius::same(8),
                Stroke::new(1.0, pal.border.gamma_multiply(a)),
                StrokeKind::Inside,
            );
        }
        let col = if hot {
            if setup {
                pal.text
            } else {
                pal.danger
            }
        } else {
            pal.muted
        };
        if setup {
            sliders_icon(p, r.center(), col.gamma_multiply(a));
        } else {
            close_icon(p, r.center(), col.gamma_multiply(a));
        }
    }
}

// ---------------------------------------------------------------- 横排（仪表盘）

/// 左侧宽度：「本轮」胶囊与「累计 · N 轮」取更宽的
fn side_width(ctx: &egui::Context, rows: &Rows) -> f32 {
    let pill = 11.0 * 2.0 + text_w(ctx, rows.turn_label, 11.5, FONT_SB);
    let total = text_w(ctx, rows.total_label, 10.5, FONT_R);
    pill.max(total)
}

fn suffix_w(ctx: &egui::Context, c: &Chip, size: f32, gap: f32) -> f32 {
    if c.suffix.is_empty() {
        0.0
    } else {
        gap + text_w(ctx, &c.suffix, size, FONT_SB)
    }
}

/// 一列的内容宽：标签 / 本轮值+后缀 / 累计值+后缀 取最宽
fn col_content_w(ctx: &egui::Context, rows: &Rows, i: usize) -> f32 {
    let c1 = &rows.turn[i];
    let mut w = text_w(ctx, c1.label, 9.5, FONT_R)
        .max(text_w(ctx, &c1.value, VALUE_H, FONT_SB) + suffix_w(ctx, c1, 10.0, 3.0));
    if let Some(c2) = rows.total.get(i) {
        w = w.max(text_w(ctx, &c2.value, SUB_H, FONT_R) + suffix_w(ctx, c2, 9.5, 2.0));
    }
    w
}

fn metrics_x(ctx: &egui::Context, rows: &Rows) -> f32 {
    PAD_L + side_width(ctx, rows) + 18.0
}

pub fn measure_horizontal(ctx: &egui::Context, rows: &Rows) -> egui::Vec2 {
    let mut total = metrics_x(ctx, rows);
    for i in 0..rows.turn.len() {
        total += col_content_w(ctx, rows, i) + COL_PAD * 2.0;
    }
    vec2(total.max(320.0) + BTN_ZONE, BAR_H)
}

/// 左侧「本轮」胶囊：与右侧大数字同一行，生成中时用点缀色
fn turn_pill(ctx: &egui::Context, p: &Painter, pal: &Palette, top: f32, rows: &Rows, v: &View) {
    let cy = top + VALUE_CY;
    let tw = text_w(ctx, rows.turn_label, 11.5, FONT_SB);
    let r = Rect::from_min_size(
        pos2(PAD_L, cy - 12.5),
        vec2(tw + 22.0, 25.0),
    );
    let fg = if v.live || v.active {
        pal.accent
    } else {
        pal.muted
    };
    p.rect_filled(r, CornerRadius::same(8), fg.gamma_multiply(0.10));
    at(
        p,
        pos2(r.center().x, cy),
        Align2::CENTER_CENTER,
        rows.turn_label,
        11.5,
        FONT_SB,
        fg,
    );
}

/// 左上角状态点：对齐右侧列标签那一行。
/// 生成中呼吸 + 光晕，任务在跑（工具/思考）时稳定亮着，空闲才变灰。
fn status_dot_top(p: &Painter, pal: &Palette, top: f32, v: &View) {
    let pulse = if v.live {
        0.72 + 0.28 * (v.time * 4.0).sin()
    } else {
        1.0
    };
    let c = pos2(PAD_L + 3.5, top + LABEL_CY);
    if v.live {
        p.circle_filled(c, 5.5, pal.accent.gamma_multiply(0.10 * pulse));
    }
    let color = if v.live || v.active {
        pal.accent
    } else {
        pal.faint
    };
    p.circle_filled(c, 2.5, color.gamma_multiply(pulse));
}

pub fn draw_horizontal(ctx: &egui::Context, p: &Painter, body: Rect, v: &View) {
    let rows = &v.rows;
    let pal = v.pal;
    let top = body.top();

    turn_pill(ctx, p, pal, top, rows, v);
    status_dot_top(p, pal, top, v);
    label(
        p,
        PAD_L,
        top + SUB_CY,
        rows.total_label,
        10.5,
        pal.muted,
    );

    let sep_top = top + 16.0;
    let sep_bottom = body.bottom() - 16.0;
    let mut edge = metrics_x(ctx, rows);
    for (i, c1) in rows.turn.iter().enumerate() {
        if i > 0 {
            p.line_segment(
                [pos2(edge, sep_top), pos2(edge, sep_bottom)],
                Stroke::new(1.0, pal.divider),
            );
        }
        let x = edge + COL_PAD;
        label(p, x, top + LABEL_CY, c1.label, 9.5, pal.faint);
        number(p, x, top + VALUE_CY, &c1.value, VALUE_H, c1.color);
        if !c1.suffix.is_empty() {
            chips::draw_suffix(ctx, p, x, top + VALUE_CY, &c1.value, VALUE_H, &c1.suffix, 10.0, 3.0, 0.5, 0.85, pal);
        }
        if let Some(c2) = rows.total.get(i) {
            let vc = if c2.value == "—" { pal.faint } else { pal.muted };
            at(
                p,
                pos2(x, top + SUB_CY),
                Align2::LEFT_CENTER,
                &c2.value,
                SUB_H,
                FONT_R,
                vc,
            );
            if !c2.suffix.is_empty() {
                let vw = text_w(ctx, &c2.value, SUB_H, FONT_R);
                at(
                    p,
                    pos2(x + vw + 2.0, top + SUB_CY + 0.5),
                    Align2::LEFT_CENTER,
                    &c2.suffix,
                    9.5,
                    FONT_SB,
                    pal.accent2.gamma_multiply(0.6),
                );
            }
        }
        edge += col_content_w(ctx, rows, i) + COL_PAD * 2.0;
    }
}

// ---------------------------------------------------------------- 竖排

struct Vertical {
    c1_x: f32,
    c2_x: f32,
}

fn vertical(ctx: &egui::Context, rows: &Rows) -> (Vertical, egui::Vec2) {
    let label_w = 44.0;
    let mut c1 = 0.0f32;
    let mut c2 = 0.0f32;
    for (i, c) in rows.turn.iter().enumerate() {
        c1 = c1.max(chips::width(ctx, c, VALUE_V_H));
        if let Some(c) = rows.total.get(i) {
            c2 = c2.max(chips::width(ctx, c, VALUE_V_V));
        }
    }
    let c1_x = PAD_L + 11.0 + label_w;
    let c1w = c1.max(text_w(ctx, "本轮", 10.5, FONT_SB) + 26.0) + 12.0;
    let c2_x = c1_x + c1w + 20.0;
    let c2w = c2.max(text_w(ctx, rows.total_label, 10.0, FONT_R)) + 6.0;
    let width = (15.0 + label_w + 12.0 + c1w + 20.0 + c2w + 15.0 + BTN_ZONE).max(240.0);
    let n = rows.turn.len().max(1) as f32;
    let height = 12.0 + 22.0 + 6.0 + n * 24.0 + 10.0;
    (Vertical { c1_x, c2_x }, vec2(width, height))
}

const VALUE_V_H: f32 = 14.0;
const VALUE_V_V: f32 = 12.0;

pub fn measure_vertical(ctx: &egui::Context, rows: &Rows) -> egui::Vec2 {
    vertical(ctx, rows).1
}

pub fn draw_vertical(ctx: &egui::Context, p: &Painter, body: Rect, v: &View) {
    let rows = &v.rows;
    let pal = v.pal;
    let head_cy = body.top() + 23.0;
    status_dot(p, pal, head_cy, v);

    let (layout, _) = vertical(ctx, rows);
    // 列表头：本轮用胶囊、累计用灰字，与横排同一语言
    let tw = text_w(ctx, rows.turn_label, 10.5, FONT_SB);
    let pill = Rect::from_min_size(
        pos2(layout.c1_x, head_cy - 9.5),
        vec2(tw + 18.0, 19.0),
    );
    let fg = if v.live || v.active {
        pal.accent
    } else {
        pal.muted
    };
    p.rect_filled(pill, CornerRadius::same(6), fg.gamma_multiply(0.10));
    at(
        p,
        pos2(layout.c1_x + 9.0, head_cy),
        Align2::LEFT_CENTER,
        rows.turn_label,
        10.5,
        FONT_SB,
        fg,
    );
    at(
        p,
        pos2(layout.c2_x, head_cy),
        Align2::LEFT_CENTER,
        rows.total_label,
        10.0,
        FONT_R,
        pal.muted,
    );
    p.line_segment(
        [
            pos2(PAD_L, head_cy + 14.0),
            pos2(body.right() - BTN_ZONE + 6.0, head_cy + 14.0),
        ],
        Stroke::new(1.0, pal.divider),
    );

    let row0 = body.top() + 44.0;
    for (i, c) in rows.turn.iter().enumerate() {
        let cy = row0 + i as f32 * 24.0 + 12.0;
        label(p, PAD_L + 11.0, cy, c.label, 10.0, pal.muted);
        chips::draw_value(p, layout.c1_x, cy, &c.value, VALUE_V_H, c.color);
        if !c.suffix.is_empty() {
            chips::draw_suffix(
                ctx,
                p,
                layout.c1_x,
                cy,
                &c.value,
                VALUE_V_H,
                &c.suffix,
                9.5,
                4.0,
                0.0,
                0.85,
                pal,
            );
        }
        if let Some(c2) = rows.total.get(i) {
            let vc = if c2.value == "—" {
                pal.faint
            } else {
                pal.muted
            };
            chips::draw_value(p, layout.c2_x, cy, &c2.value, VALUE_V_V, vc);
            if !c2.suffix.is_empty() {
                chips::draw_suffix(
                    ctx,
                    p,
                    layout.c2_x,
                    cy,
                    &c2.value,
                    VALUE_V_V,
                    &c2.suffix,
                    9.0,
                    3.0,
                    0.0,
                    0.6,
                    pal,
                );
            }
        }
    }
}

// ---------------------------------------------------------------- 公共小件

/// 竖排左上角的状态点：生成中呼吸，任务在跑时稳定亮着，空闲才变灰
fn status_dot(p: &Painter, pal: &Palette, cy: f32, v: &View) {
    let pulse = if v.live {
        0.72 + 0.28 * (v.time * 4.0).sin()
    } else {
        1.0
    };
    let dot = if v.live || v.active {
        pal.accent
    } else {
        pal.faint
    };
    if v.live {
        p.circle_filled(
            pos2(PAD_L + 2.5, cy),
            6.0,
            pal.accent.gamma_multiply(0.10 * pulse),
        );
    }
    p.circle_filled(pos2(PAD_L + 2.5, cy), 2.4, dot.gamma_multiply(pulse));
}
