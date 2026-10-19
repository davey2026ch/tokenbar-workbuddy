//! 一个指标 = 左侧小标签 + 右侧数值（+ 可选的缓存命中率后缀）。

use super::text::{at, number, text_w, FONT_R, FONT_SB};
use super::theme::Palette;
use crate::data::Stats;
use crate::metric::{Metric, Tone};
use eframe::egui::{self, pos2, Align2, Color32, Painter};

pub struct Chip {
    pub label: &'static str,
    pub value: String,
    pub suffix: String,
    pub color: Color32,
}

/// 按用户排序好的指标，把一组统计值变成可绘制的卡片内容
pub fn build(stats: Option<&Stats>, metrics: &[Metric], pal: &Palette) -> Vec<Chip> {
    let empty = Stats::default();
    let known = stats.is_some();
    let st = stats.unwrap_or(&empty);
    metrics
        .iter()
        .map(|m| Chip {
            label: m.short_label(),
            value: m.value(st, known),
            suffix: m.suffix(st),
            color: color(m.tone(st), pal),
        })
        .collect()
}

fn color(tone: Tone, pal: &Palette) -> Color32 {
    match tone {
        Tone::Normal => pal.text,
        Tone::Accent => pal.accent,
        Tone::Cache => pal.accent2,
        Tone::Faint => pal.faint,
        Tone::Warm => pal.warm,
    }
}

/// 竖排测量用的宽度估算：标签 + 数值 + 后缀
pub fn width(ctx: &egui::Context, c: &Chip, value_size: f32) -> f32 {
    let lw = text_w(ctx, c.label, 10.0, FONT_R);
    let vw = text_w(ctx, &c.value, value_size, FONT_SB);
    let sw = if c.suffix.is_empty() {
        0.0
    } else {
        4.0 + text_w(ctx, &c.suffix, 9.5, FONT_SB)
    };
    lw + 6.0 + vw + sw
}

pub fn draw_value(p: &Painter, x: f32, cy: f32, value: &str, size: f32, color: Color32) {
    number(p, x, cy, value, size, color);
}

/// 数值后面的小字（缓存命中率）：数值宽度是量出来的，所以要 ctx
#[allow(clippy::too_many_arguments)]
pub fn draw_suffix(
    ctx: &egui::Context,
    p: &Painter,
    value_x: f32,
    cy: f32,
    value: &str,
    value_size: f32,
    suffix: &str,
    size: f32,
    gap: f32,
    dy: f32,
    alpha: f32,
    pal: &Palette,
) {
    let vw = text_w(ctx, value, value_size, FONT_SB);
    at(
        p,
        pos2(value_x + vw + gap, cy + dy),
        Align2::LEFT_CENTER,
        suffix,
        size,
        FONT_SB,
        pal.accent2.gamma_multiply(alpha),
    );
}
