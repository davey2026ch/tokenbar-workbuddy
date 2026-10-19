//! 配色：深/浅两套调色板 + 颜色混合小工具。

use eframe::egui::Color32;

pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub surface: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub warm: Color32,
    pub danger: Color32,
    pub divider: Color32,
    pub hover: Color32,
    pub track: Color32,
    pub shadow: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            bg: Color32::from_rgba_unmultiplied(12, 15, 21, 251),
            surface: Color32::from_rgba_unmultiplied(255, 255, 255, 14),
            border: Color32::from_rgba_unmultiplied(255, 255, 255, 26),
            text: Color32::from_rgb(233, 238, 248),
            muted: Color32::from_rgb(140, 150, 168),
            faint: Color32::from_rgb(96, 105, 122),
            accent: Color32::from_rgb(59, 227, 196),
            accent2: Color32::from_rgb(107, 160, 255),
            warm: Color32::from_rgb(240, 180, 94),
            danger: Color32::from_rgb(255, 122, 128),
            divider: Color32::from_rgba_unmultiplied(255, 255, 255, 20),
            hover: Color32::from_rgba_unmultiplied(255, 255, 255, 26),
            track: Color32::from_rgba_unmultiplied(255, 255, 255, 16),
            shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 90),
        }
    } else {
        Palette {
            dark,
            bg: Color32::from_rgba_unmultiplied(255, 255, 255, 253),
            surface: Color32::from_rgba_unmultiplied(15, 23, 42, 8),
            border: Color32::from_rgba_unmultiplied(15, 23, 42, 26),
            text: Color32::from_rgb(17, 24, 39),
            muted: Color32::from_rgb(108, 117, 133),
            faint: Color32::from_rgb(150, 158, 172),
            accent: Color32::from_rgb(13, 158, 132),
            accent2: Color32::from_rgb(44, 107, 224),
            warm: Color32::from_rgb(190, 116, 18),
            danger: Color32::from_rgb(214, 58, 66),
            divider: Color32::from_rgba_unmultiplied(15, 23, 42, 22),
            hover: Color32::from_rgba_unmultiplied(15, 23, 42, 20),
            track: Color32::from_rgba_unmultiplied(15, 23, 42, 14),
            shadow: Color32::from_rgba_unmultiplied(15, 23, 42, 60),
        }
    }
}

/// 线性混合两个颜色（t=0 取 a，t=1 取 b）
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), 255)
}

/// 设置窗口的页面底色：浅色下比卡片略灰，让白色卡片浮出来
pub fn page_bg(pal: &Palette) -> Color32 {
    if pal.dark {
        pal.bg
    } else {
        Color32::from_rgb(246, 247, 250)
    }
}

/// 设置窗口里卡片/控件的底色
pub fn card_bg(pal: &Palette) -> Color32 {
    if pal.dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 12)
    } else {
        Color32::WHITE
    }
}

/// 强调色底上的前景色（对勾、主按钮文字）
pub fn on_accent(pal: &Palette) -> Color32 {
    if pal.dark {
        Color32::from_rgb(8, 14, 18)
    } else {
        Color32::WHITE
    }
}
