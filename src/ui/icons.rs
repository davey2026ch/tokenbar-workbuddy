//! 自绘的小图标：设置滑杆、关闭叉、布局缩略图、开关、对勾，以及程序图标。

use super::theme::Palette;
use eframe::egui::{pos2, vec2, Color32, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind};

/// 三行"滑杆"图标：设置
pub fn sliders_icon(p: &Painter, c: Pos2, color: Color32) {
    let w = 5.0;
    let rows = [-4.0f32, 0.0, 4.0];
    for (i, dy) in rows.iter().enumerate() {
        let y = c.y + dy;
        p.line_segment(
            [pos2(c.x - w, y), pos2(c.x + w, y)],
            Stroke::new(1.2, color),
        );
        let kx = c.x + if i == 1 { 2.4 } else { -2.4 };
        p.circle_filled(pos2(kx, y), 1.7, color);
        p.circle_filled(pos2(kx, y), 0.9, Color32::TRANSPARENT);
    }
}

/// 关闭：一个叉
pub fn close_icon(p: &Painter, c: Pos2, color: Color32) {
    let s = 4.2;
    p.line_segment(
        [pos2(c.x - s, c.y - s), pos2(c.x + s, c.y + s)],
        Stroke::new(1.5, color),
    );
    p.line_segment(
        [pos2(c.x + s, c.y - s), pos2(c.x - s, c.y + s)],
        Stroke::new(1.5, color),
    );
}

/// 布局缩略图：横排画一条两行的小横条，竖排画一个两列的小方块
pub fn draw_layout_thumb(p: &Painter, card: Rect, vertical: bool, pal: &Palette, on: bool) {
    let top = card.top() + 10.0;
    let area = Rect::from_min_max(
        pos2(card.left() + 10.0, top),
        pos2(card.right() - 10.0, top + 30.0),
    );
    let tone = if on { pal.accent } else { pal.faint };
    if vertical {
        let w = 40.0f32.min(area.width());
        let r = Rect::from_center_size(area.center(), vec2(w, 30.0));
        p.rect_filled(r, CornerRadius::same(4), pal.surface);
        p.rect_stroke(
            r,
            CornerRadius::same(4),
            Stroke::new(1.0, tone.gamma_multiply(0.5)),
            StrokeKind::Inside,
        );
        for i in 0..3 {
            let y = r.top() + 7.0 + i as f32 * 7.0;
            p.rect_filled(
                Rect::from_min_size(pos2(r.left() + 4.0, y), vec2(8.0, 2.0)),
                CornerRadius::same(1),
                tone.gamma_multiply(0.55),
            );
            p.rect_filled(
                Rect::from_min_size(pos2(r.left() + 16.0, y), vec2(14.0, 2.0)),
                CornerRadius::same(1),
                tone,
            );
        }
    } else {
        let r = Rect::from_center_size(area.center(), vec2(area.width(), 30.0));
        p.rect_filled(r, CornerRadius::same(4), pal.surface);
        p.rect_stroke(
            r,
            CornerRadius::same(4),
            Stroke::new(1.0, tone.gamma_multiply(0.5)),
            StrokeKind::Inside,
        );
        for row in 0..2 {
            let y = r.top() + 8.0 + row as f32 * 8.0;
            let mut x = r.left() + 6.0;
            for k in 0..4 {
                let w = 12.0 + (k % 2) as f32 * 6.0;
                p.rect_filled(
                    Rect::from_min_size(pos2(x, y), vec2(w, 2.4)),
                    CornerRadius::same(1),
                    if row == 0 {
                        tone
                    } else {
                        tone.gamma_multiply(0.5)
                    },
                );
                x += w + 5.0;
                if x > r.right() - 8.0 {
                    break;
                }
            }
        }
    }
}

/// 开关
pub fn switch(p: &Painter, r: Rect, on: bool, pal: &Palette) {
    let bg = if on {
        pal.accent.gamma_multiply(if pal.dark { 0.85 } else { 0.9 })
    } else {
        pal.track
    };
    p.rect_filled(r, CornerRadius::same((r.height() / 2.0) as u8), bg);
    let kr = r.height() / 2.0 - 2.0;
    let kx = if on {
        r.right() - kr - 2.0
    } else {
        r.left() + kr + 2.0
    };
    p.circle_filled(
        pos2(kx, r.center().y),
        kr,
        if on { Color32::WHITE } else { pal.faint },
    );
}

/// 对勾
pub fn check(p: &Painter, c: Pos2, color: Color32) {
    p.line_segment(
        [pos2(c.x - 3.6, c.y + 0.2), pos2(c.x - 1.0, c.y + 2.8)],
        Stroke::new(1.8, color),
    );
    p.line_segment(
        [pos2(c.x - 1.0, c.y + 2.8), pos2(c.x + 3.8, c.y - 2.8)],
        Stroke::new(1.8, color),
    );
}

/// 程序图标（64 像素，运行时画一个"迷你指标条"）
pub fn app_icon() -> eframe::egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    let stops = [(59u8, 227u8, 196u8), (107, 160, 255), (255, 138, 190)];
    let half = (n as f32 - 1.0) * 0.5;
    let r = 15.0f32;
    for y in 0..n {
        for x in 0..n {
            let cx = x as f32 - half;
            let cy = y as f32 - half;
            let qx = (cx.abs() - (half - r)).max(0.0);
            let qy = (cy.abs() - (half - r)).max(0.0);
            if qx * qx + qy * qy > r * r {
                continue;
            }
            let mut c = (16u8, 20u8, 28u8);
            if y < 9 {
                let t = x as f32 / (n as f32 - 1.0);
                let seg = t * (stops.len() - 1) as f32;
                let k = (seg.floor() as usize).min(stops.len() - 2);
                let f = seg - k as f32;
                let (a, b) = (stops[k], stops[k + 1]);
                c = (
                    (a.0 as f32 + (b.0 as f32 - a.0 as f32) * f) as u8,
                    (a.1 as f32 + (b.1 as f32 - a.1 as f32) * f) as u8,
                    (a.2 as f32 + (b.2 as f32 - a.2 as f32) * f) as u8,
                );
            } else if (24..31).contains(&y) && (12..52).contains(&x) {
                c = (233, 238, 248);
            } else if (38..45).contains(&y) && (12..44).contains(&x) {
                c = (140, 150, 168);
            }
            let i = (y * n + x) * 4;
            rgba[i] = c.0;
            rgba[i + 1] = c.1;
            rgba[i + 2] = c.2;
            rgba[i + 3] = 255;
        }
    }
    eframe::egui::IconData {
        rgba,
        width: n as u32,
        height: n as u32,
    }
}
