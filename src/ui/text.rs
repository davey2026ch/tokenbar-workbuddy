//! 文字绘制小工具：两个字体族 + 常用对齐方式。

use eframe::egui::{self, pos2, Align2, Color32, FontFamily, FontId, Painter, Pos2};

pub const FONT_R: &str = "regular";
pub const FONT_SB: &str = "sb";

pub fn fam(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

/// 量一段文字的宽度（egui 的字体测量必须走 fonts_mut）
pub fn text_w(ctx: &egui::Context, text: &str, size: f32, family: &str) -> f32 {
    ctx.fonts_mut(|f| {
        f.layout_no_wrap(
            text.to_owned(),
            FontId::new(size, fam(family)),
            Color32::WHITE,
        )
        .size()
        .x
    })
}

pub fn at(
    p: &Painter,
    pos: Pos2,
    align: Align2,
    text: &str,
    size: f32,
    family: &str,
    color: Color32,
) {
    p.text(pos, align, text, FontId::new(size, fam(family)), color);
}

/// 左对齐、垂直居中（以 y 为中心）
pub fn label(p: &Painter, x: f32, cy: f32, text: &str, size: f32, color: Color32) {
    at(
        p,
        pos2(x, cy),
        Align2::LEFT_CENTER,
        text,
        size,
        FONT_R,
        color,
    );
}

/// 同上，用加粗字体（数值）
pub fn number(p: &Painter, x: f32, cy: f32, text: &str, size: f32, color: Color32) {
    at(
        p,
        pos2(x, cy),
        Align2::LEFT_CENTER,
        text,
        size,
        FONT_SB,
        color,
    );
}
