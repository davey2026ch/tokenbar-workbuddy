//! 更新提示：贴在 bar 正上方的一张小卡片（自己的 OS 窗口，不抢焦点）。
//!
//! 三种样子，由 `update::Status` 决定：
//! - 发现新版：一句"发现新版本 v1.5.0"，整卡可点 → 开始下载
//! - 下载中：进度条 + 已下载多少
//! - 已就绪："重启后生效"，按钮是「立即重启」

use crate::ui::{card, mix, on_accent, palette, text_at, text_w, Palette, FONT_R, FONT_SB};
use crate::update::Status;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke,
};
use std::sync::{Arc, Mutex};

pub const W: f32 = 366.0;
pub const H: f32 = 60.0;

const PAD: f32 = 14.0;
const BTN_W: f32 = 78.0;
const BTN_H: f32 = 28.0;
const CLOSE: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// 开始下载
    Download,
    /// 重启到新版本
    Restart,
    /// 这次先不更新
    Dismiss,
    /// 失败了，再试一次
    Retry,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Kind {
    #[default]
    Update,
    Ready,
    Failed,
}

/// 这一帧要显示什么（由 App 每帧从 `Status` 翻出来）
#[derive(Clone, Default)]
pub struct View {
    pub kind: Kind,
    pub title: String,
    pub sub: String,
    pub button: Option<&'static str>,
    pub progress: Option<f32>,
    pub closable: bool,
}

impl View {
    /// 只有"该让用户看见"的状态才返回 Some
    pub fn from_status(s: &Status) -> Option<Self> {
        match s {
            Status::Found(rel) => Some(Self {
                kind: Kind::Update,
                title: format!("发现新版本 v{}", rel.version),
                sub: "点一下开始下载，下载完再重启".into(),
                button: Some("更新"),
                progress: None,
                closable: true,
            }),
            Status::Downloading { got, total } => Some(Self {
                kind: Kind::Update,
                title: "正在下载新版本…".into(),
                sub: format!(
                    "{} / {}",
                    mib(*got),
                    total.map(mib).unwrap_or_else(|| "?".into())
                ),
                button: None,
                progress: Some(match total {
                    Some(t) if *t > 0 => (*got as f32 / *t as f32).clamp(0.0, 1.0),
                    _ => 0.0,
                }),
                closable: false,
            }),
            Status::Ready { version } => Some(Self {
                kind: Kind::Ready,
                title: format!("v{version} 已下载"),
                sub: "关闭并重新打开 Tokenbar 才会生效".into(),
                button: Some("立即重启"),
                progress: None,
                closable: false,
            }),
            Status::Failed(msg) => Some(Self {
                kind: Kind::Failed,
                title: "更新没成功".into(),
                sub: msg.clone(),
                button: Some("重试"),
                progress: None,
                closable: true,
            }),
            Status::Idle | Status::Checking | Status::Latest | Status::Manual(_) => None,
        }
    }
}

/// 跟界面共享的状态
pub struct Shared {
    pub open: bool,
    pub dark: bool,
    pub view: View,
    /// 窗口左上角（逻辑坐标），变了才下发
    pub pos: Pos2,
    pub first: bool,
    pub action: Option<Action>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            open: false,
            dark: true,
            view: View::default(),
            pos: pos2(0.0, 0.0),
            first: true,
            action: None,
        }
    }
}

pub fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("tokenbar-notice")
}

pub fn show(ctx: &egui::Context, shared: Arc<Mutex<Shared>>) {
    let (open, first, pos) = {
        let Ok(d) = shared.lock() else {
            return;
        };
        (d.open, d.first, d.pos)
    };
    if !open {
        return;
    }

    let mut vb = egui::ViewportBuilder::default()
        .with_title("Tokenbar 更新")
        .with_inner_size([W, H])
        .with_min_inner_size([W, H])
        .with_max_inner_size([W, H])
        .with_resizable(false)
        .with_decorations(false)
        .with_transparent(true)
        .with_has_shadow(false)
        .with_taskbar(false)
        .with_always_on_top()
        // 不能抢焦点：用户可能正在 WorkBuddy 里打字
        .with_active(false);
    if first {
        vb = vb.with_position(pos);
        if let Ok(mut d) = shared.lock() {
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
        // 跟着 bar 走：位置变了才下发（bar 被拖动 / 换布局都会变）
        if let Some(outer) = ctx.input(|i| i.viewport().outer_rect) {
            if (outer.min - d.pos).length() > 0.5 {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(d.pos));
            }
        }
        // 延迟视口里不能再借 ctx.root_ui，自己铺满窗口造一个
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

fn draw(ui: &mut egui::Ui, d: &mut Shared) {
    let pal = palette(d.dark);
    let area = Rect::from_min_size(ui.max_rect().min, vec2(W, H));
    let p = ui.painter().clone();

    // 卡片：跟 bar 一样的投影 + 圆角，视觉上是一家人
    card(&p, area, &pal, 13);
    if d.view.kind == Kind::Update {
        // 一点强调色洗底，提示"这里有事要做"
        p.rect_filled(area, CornerRadius::same(13), pal.accent.gamma_multiply(0.045));
    }

    icon(&p, &pal, area, d.view.kind);

    let text_x = area.left() + 50.0;
    let right = area.right() - PAD;
    let btn_w = if d.view.button.is_some() { BTN_W } else { 0.0 };
    let close_w = if d.view.closable { CLOSE + 12.0 } else { 0.0 };
    let text_max = (right - btn_w - close_w - text_x - 6.0).max(0.0);

    text_at(
        &p,
        pos2(text_x, area.top() + 22.0),
        Align2::LEFT_CENTER,
        &fit(ui.ctx(), &d.view.title, text_max, 12.5, FONT_SB),
        12.5,
        FONT_SB,
        pal.text,
    );
    text_at(
        &p,
        pos2(text_x, area.top() + 39.0),
        Align2::LEFT_CENTER,
        &fit(ui.ctx(), &d.view.sub, text_max, 10.0, FONT_R),
        10.0,
        FONT_R,
        if d.view.kind == Kind::Failed {
            pal.danger
        } else {
            pal.faint
        },
    );

    // 进度条
    if let Some(t) = d.view.progress {
        let bar = Rect::from_min_size(
            pos2(area.left() + PAD, area.bottom() - 9.0),
            vec2(area.width() - PAD * 2.0, 3.0),
        );
        p.rect_filled(bar, CornerRadius::same(2), pal.track);
        if t > 0.01 {
            let fill = Rect::from_min_size(bar.min, vec2(bar.width() * t, bar.height()));
            p.rect_filled(fill, CornerRadius::same(2), pal.accent);
        }
    }

    // 先登记整卡（后登记的盖在上面，所以按钮 / 关闭要放在后面）
    let whole = ui.interact(area, id_of("card"), Sense::click());
    let mut hot = whole.hovered() && d.view.button.is_some();

    // 关闭（这次先不更新）
    let mut dismissed = false;
    if d.view.closable {
        let close_r = Rect::from_center_size(
            pos2(right - btn_w - 10.0 - CLOSE * 0.5, area.center().y),
            vec2(CLOSE, CLOSE),
        );
        let resp = ui.interact(close_r.expand(5.0), id_of("close"), Sense::click());
        if resp.hovered() {
            p.circle_filled(close_r.center(), 9.0, pal.hover);
            hot = true;
        }
        let c = close_r.center();
        let col = if resp.hovered() { pal.text } else { pal.muted };
        let r = CLOSE * 0.28;
        p.line_segment(
            [pos2(c.x - r, c.y - r), pos2(c.x + r, c.y + r)],
            Stroke::new(1.5, col),
        );
        p.line_segment(
            [pos2(c.x + r, c.y - r), pos2(c.x - r, c.y + r)],
            Stroke::new(1.5, col),
        );
        if resp.clicked() {
            d.action = Some(Action::Dismiss);
            dismissed = true;
        }
    }

    // 按钮
    if let Some(label) = d.view.button {
        let btn = Rect::from_min_size(
            pos2(right - btn_w, area.center().y - BTN_H * 0.5),
            vec2(btn_w, BTN_H),
        );
        let resp = ui.interact(btn, id_of("btn"), Sense::click());
        if resp.hovered() {
            hot = true;
        }
        let bg = if resp.hovered() {
            mix(pal.accent, Color32::WHITE, 0.12)
        } else {
            pal.accent
        };
        p.rect_filled(btn, CornerRadius::same(8), bg);
        text_at(
            &p,
            btn.center(),
            Align2::CENTER_CENTER,
            label,
            11.5,
            FONT_R,
            on_accent(&pal),
        );
        if resp.clicked() {
            d.action = Some(primary(d.view.kind));
            dismissed = true; // 免得整卡再触发一次
        }
    }

    if whole.clicked() && d.view.button.is_some() && !dismissed {
        d.action = Some(primary(d.view.kind));
    }

    ui.ctx().set_cursor_icon(if hot {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::Default
    });
}

fn primary(kind: Kind) -> Action {
    match kind {
        Kind::Ready => Action::Restart,
        Kind::Failed => Action::Retry,
        Kind::Update => Action::Download,
    }
}

/// 左侧图标：下载箭头 / 对勾 / 感叹号
fn icon(p: &egui::Painter, pal: &Palette, area: Rect, kind: Kind) {
    let c = pos2(area.left() + PAD + 13.0, area.center().y);
    let (fg, bg) = match kind {
        Kind::Ready => (pal.accent, pal.accent.gamma_multiply(0.14)),
        Kind::Failed => (pal.danger, pal.danger.gamma_multiply(0.14)),
        Kind::Update => (pal.accent2, pal.accent2.gamma_multiply(0.14)),
    };
    p.circle_filled(c, 13.0, bg);
    match kind {
        Kind::Ready => {
            // 对勾
            p.line_segment(
                [pos2(c.x - 5.0, c.y), pos2(c.x - 1.4, c.y + 3.6)],
                Stroke::new(1.8, fg),
            );
            p.line_segment(
                [pos2(c.x - 1.4, c.y + 3.6), pos2(c.x + 5.0, c.y - 3.1)],
                Stroke::new(1.8, fg),
            );
        }
        Kind::Failed => {
            p.line_segment(
                [pos2(c.x, c.y - 5.2), pos2(c.x, c.y + 1.2)],
                Stroke::new(1.8, fg),
            );
            p.circle_filled(pos2(c.x, c.y + 4.6), 1.2, fg);
        }
        Kind::Update => {
            // 向下的箭头 + 托盘
            p.line_segment(
                [pos2(c.x, c.y - 5.6), pos2(c.x, c.y + 1.6)],
                Stroke::new(1.8, fg),
            );
            p.line_segment(
                [pos2(c.x - 3.4, c.y - 1.6), pos2(c.x, c.y + 2.2)],
                Stroke::new(1.8, fg),
            );
            p.line_segment(
                [pos2(c.x + 3.4, c.y - 1.6), pos2(c.x, c.y + 2.2)],
                Stroke::new(1.8, fg),
            );
            p.line_segment(
                [pos2(c.x - 4.6, c.y + 5.4), pos2(c.x + 4.6, c.y + 5.4)],
                Stroke::new(1.8, fg),
            );
        }
    }
}

#[allow(deprecated)]
fn id_of(tag: &str) -> egui::Id {
    egui::Id::new(("tokenbar-notice", tag))
}

fn mib(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1048576.0)
    } else {
        format!("{} KB", bytes / 1024)
    }
}

/// 太长的文字按宽度裁掉，尾巴补省略号
fn fit(ctx: &egui::Context, text: &str, max_w: f32, size: f32, font: &str) -> String {
    if max_w <= 8.0 || text.is_empty() {
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
