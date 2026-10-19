#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Tokenbar —— WorkBuddy 会话指标悬浮条
// 绿色便携、跨平台（Windows / macOS），窗口无边框、置顶、可拖动。

mod config;
mod data;
mod metric;
mod notice;
mod platform;
mod settings;
mod ui;
mod update;

use config::{Config, ThemePref};
use data::{Snapshot, Tracker};
use eframe::egui;
use egui::{pos2, vec2, Pos2};
use platform::Focus;
use raw_window_handle::HasWindowHandle;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const APP_NAME: &str = "Tokenbar";
/// 启动多久后悄悄查一次更新
const CHECK_AFTER: Duration = Duration::from_secs(3);
/// 查前台应用的节奏：Windows 与 macOS 统一（见 README「跟随显隐」）
const FOCUS_INTERVAL: Duration = Duration::from_millis(500);
/// 头顶那张提示卡离卡片留的空隙
const NOTICE_GAP: f32 = 10.0;

pub struct App {
    pub cfg: Config,
    tracker: Tracker,
    pub snap: Snapshot,
    pub dark: bool,
    pub hover_t: f32,
    pub settings_open: bool,
    pub draft: Arc<Mutex<settings::Draft>>,
    /// bar 窗口的位置和尺寸
    pub win: ui::WindowState,
    pub save_at: Option<Instant>,
    pub quit: bool,
    polished: bool,
    pub visible: bool,
    launch_at: Instant,
    last_poll: Instant,
    last_focus_check: Instant,
    wb_running: bool,
    wb_fg: bool,
    wb_seen: bool,
    self_fg: bool,
    /// 自动更新
    pub updater: update::Updater,
    pub notice: Arc<Mutex<notice::Shared>>,
    /// bar 自己的窗口句柄（Windows），多屏定位要用
    pub hwnd: Option<isize>,
    /// 首次启动要落到 WorkBuddy 所在的那块屏幕（只有没保存过位置时才有值）
    pub place: Option<platform::Screen>,
    /// 已经尝试摆了几帧，防止一直跟系统抢位置
    place_frames: u8,
    /// 启动后是否已经查过
    checked: bool,
    /// 提示卡被"以后再说"关掉了吗
    notice_dismissed: bool,
    /// 用户主动点了更新/重试：出错时也把提示弹出来
    notice_pinned: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        install_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut tracker = Tracker::new_with_cache(config::stats_cache_path());
        tracker.poll();
        let snap = tracker.snapshot();
        let dark = match cfg.theme {
            ThemePref::Dark => true,
            ThemePref::Light => false,
            ThemePref::System => system_dark(&cc.egui_ctx),
        };
        let draft = Arc::new(Mutex::new(settings::Draft::new(&cfg)));
        // 没存过位置 = 第一次运行：摆到 WorkBuddy 所在的那块屏幕去
        let place = if cfg.x.is_none() {
            platform::workbuddy_screen()
        } else {
            None
        };
        Self {
            cfg,
            tracker,
            snap,
            dark,
            hover_t: 0.0,
            settings_open: false,
            draft,
            win: ui::WindowState::default(),
            save_at: None,
            quit: false,
            polished: false,
            visible: true,
            launch_at: Instant::now(),
            last_poll: Instant::now(),
            last_focus_check: Instant::now(),
            wb_running: true,
            wb_fg: false,
            wb_seen: false,
            self_fg: true,
            updater: update::Updater::new(),
            notice: Arc::new(Mutex::new(notice::Shared::default())),
            hwnd: None,
            place,
            place_frames: 0,
            checked: false,
            notice_dismissed: false,
            notice_pinned: false,
        }
    }

    fn theme_dark(&self, ctx: &egui::Context) -> bool {
        match self.cfg.theme {
            ThemePref::Dark => true,
            ThemePref::Light => false,
            ThemePref::System => system_dark(ctx),
        }
    }

    /// 把设置窗口的草稿同步进来；确定/取消在这里收口
    fn sync_settings(&mut self) {
        let Ok(mut d) = self.draft.lock() else {
            return;
        };
        match d.result {
            Some(settings::Outcome::Commit) => {
                self.cfg.apply_prefs_from(&d.cfg);
                self.cfg.save();
                close_settings(&mut d, &mut self.settings_open);
            }
            Some(settings::Outcome::Cancel) => {
                // 位置保持当前值，只回滚外观/指标
                let (x, y) = (self.cfg.x, self.cfg.y);
                self.cfg.apply_prefs_from(&d.orig);
                self.cfg.x = x;
                self.cfg.y = y;
                self.cfg.save();
                close_settings(&mut d, &mut self.settings_open);
            }
            None => {
                if d.open {
                    self.cfg.apply_prefs_from(&d.cfg);
                } else if self.settings_open {
                    self.settings_open = false;
                }
            }
        }
    }

    pub fn open_settings(&mut self, ctx: &egui::Context) {
        if self.settings_open {
            return;
        }
        let mon = self.monitor_rect(ctx);
        let bar = self.win.rect();
        let (w, h) = (settings::WIN_W, settings::WIN_H);
        let gap = 12.0;
        // 标题栏高度各机器不同，这里只粗略估一个用于选边；
        // 真正的贴边在设置窗口首帧按真实尺寸修正（见 settings::fit_around_bar）。
        let outer_h = h + 44.0;
        let outer_w = w + 16.0;
        let place = if bar.top() - gap - outer_h >= mon.top() + 8.0 {
            settings::Place::Above
        } else if bar.bottom() + gap + outer_h <= mon.bottom() - 8.0 {
            settings::Place::Below
        } else if bar.right() + gap + outer_w <= mon.right() - 8.0 {
            settings::Place::Right
        } else if bar.left() - gap - outer_w >= mon.left() + 8.0 {
            settings::Place::Left
        } else {
            settings::Place::Above
        };
        let win_pos = match place {
            settings::Place::Above => pos2(bar.center().x - w * 0.5, bar.top() - gap - outer_h),
            settings::Place::Below => pos2(bar.center().x - w * 0.5, bar.bottom() + gap),
            settings::Place::Right => pos2(bar.right() + gap, bar.center().y - h * 0.5),
            settings::Place::Left => pos2(bar.left() - gap - outer_w, bar.center().y - h * 0.5),
        };
        if let Ok(mut d) = self.draft.lock() {
            d.open = true;
            d.first = true;
            d.focus = true;
            d.fit = true;
            d.cfg = self.cfg.clone();
            d.orig = self.cfg.clone();
            d.result = None;
            d.reset_drag();
            d.bar_rect = bar;
            d.mon_rect = mon;
            d.place = place;
            d.win_pos = pos2(win_pos.x.round(), win_pos.y.round());
        }
        self.settings_open = true;
    }

    /// bar 现在所在那块屏幕的矩形（逻辑点；副屏左上角不是 0,0）。
    ///
    /// egui 的点 = 物理像素 / 当前缩放，所以这里也用同一个缩放换算，
    /// 多屏、不同缩放都能对上。
    pub fn monitor_rect(&self, ctx: &egui::Context) -> egui::Rect {
        let pp = ctx.pixels_per_point();
        if let Some(s) = self.hwnd.and_then(platform::window_screen) {
            let (x, y, w, h) = s.to_points(pp);
            return egui::Rect::from_min_size(pos2(x, y), vec2(w, h));
        }
        let size = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(vec2(1920.0, 1080.0));
        egui::Rect::from_min_size(Pos2::ZERO, size)
    }

    /// 第一次运行时把卡片摆到 WorkBuddy 所在的那块屏幕：水平居中、离底留一段。
    ///
    /// 用系统级的物理坐标直接挪窗口，而不是给 egui 发逻辑点——跨屏缩放不一样时，
    /// 逻辑点要先按当前缩放换算，换过去之后缩放又变了，容易越算越偏。
    fn place_on_screen(&mut self) {
        let Some(screen) = self.place else {
            return;
        };
        if !self.visible {
            return; // 该隐身的时候别把它拽出来
        }
        let Some(hwnd) = self.hwnd else {
            return;
        };
        self.place_frames += 1;
        if self.place_frames > 20 {
            // 系统一直不让动（比如被某个壳接管）：放弃，保持原样
            self.place = None;
            return;
        }
        let Some((x, y, w, h)) = platform::window_rect(hwnd) else {
            self.place = None;
            return;
        };
        let margin = ui::BOTTOM_MARGIN * screen.scale;
        let (tx, ty) = platform::place_in(screen, w, h, margin);
        if (tx - x).abs() <= 2.0 && (ty - y).abs() <= 2.0 {
            // 到位了：按这块屏幕的缩放把物理坐标写回配置（egui 的点 = 物理 / 缩放）
            let pp = platform::window_screen(hwnd)
                .map(|s| s.scale)
                .unwrap_or(screen.scale);
            let (px, py) = (tx / pp, ty / pp);
            self.cfg.set_position(px, py);
            self.cfg.save();
            self.win.adopt(pos2(px, py), self.win.view_size());
            self.place = None;
            return;
        }
        platform::move_window(hwnd, tx, ty);
    }

    fn tick(&mut self, ctx: &egui::Context) {
        self.sync_settings();
        self.dark = self.theme_dark(ctx);
        if let Ok(mut d) = self.draft.lock() {
            d.dark = self.dark;
            // 设置窗口开着时 bar 可能改尺寸/被拖动，跟着重新贴边，别压住 bar
            if d.open && self.win.view_size().x > 0.0 {
                let bar = self.win.rect();
                d.mon_rect = self.monitor_rect(ctx);
                if (bar.min - d.bar_rect.min).length() > 0.5
                    || (bar.max - d.bar_rect.max).length() > 0.5
                {
                    d.bar_rect = bar;
                    d.fit = true;
                    // 延迟视口不会自己重绘，得主动叫一次，贴边才会生效
                    ctx.request_repaint_of(settings::viewport_id());
                }
            }
        }

        // 再次双击 exe 时，把已经隐藏的自己叫出来
        if wake_requested() {
            self.launch_at = Instant::now();
        }

        if self.last_poll.elapsed() > Duration::from_millis(250) {
            self.last_poll = Instant::now();
            self.tracker.poll();
            self.snap = self.tracker.snapshot();
        }

        let focus_interval = FOCUS_INTERVAL;
        if self.last_focus_check.elapsed() > focus_interval {
            self.last_focus_check = Instant::now();
            let f = platform::focus();
            self.wb_fg = f == Focus::WorkBuddy;
            self.self_fg = f == Focus::SelfApp;
            if self.wb_fg {
                self.wb_seen = true;
            }
            self.wb_running = platform::workbuddy_running();
        }

        // 跟随 WorkBuddy 显示 / 隐藏
        let grace = self.launch_at.elapsed() < Duration::from_secs(5);
        // 还没见过 WorkBuddy 在前台之前，不主动隐身（避免刚启动就消失）
        let want = !self.cfg.follow
            || !self.wb_running
            || grace
            || self.settings_open
            || !self.wb_seen
            || self.wb_fg
            || self.self_fg;
        // 用"移出屏幕"代替 ShowWindow 隐藏：
        // Windows 下 Visible(false→true) 之后窗口会收不到鼠标事件，
        // 移出屏幕不会打断输入状态，回来即可正常点击。
        if want != self.visible {
            self.visible = want;
            self.win.move_offscreen(ctx, !want);
            if want {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                    egui::WindowLevel::AlwaysOnTop,
                ));
            }
        }

        // 开机 3 秒后安静地查一次（正好等窗口摆好、字体加载完）
        if !self.checked && self.launch_at.elapsed() >= CHECK_AFTER {
            self.checked = true;
            self.updater.check(ctx.clone());
        }
        self.sync_update(ctx);
    }

    /// 把更新状态同步到"提示卡"和"设置窗口的版本信息"，并处理用户点击
    fn sync_update(&mut self, ctx: &egui::Context) {
        let st = self.updater.status();
        let view = notice::View::from_status(&st);

        // 只有用户主动点过更新，失败才弹提示；开机那次静默检查失败不打扰。
        // 设置窗口开着时不弹：版本信息就在设置里，弹出来反而压住底部按钮。
        let show = !self.settings_open && match &st {
            update::Status::Found(_) => !self.notice_dismissed,
            update::Status::Downloading { .. } => true,
            // 已经下好了，哪怕之前点了"以后再说"也要让人知道
            update::Status::Ready { .. } => true,
            update::Status::Failed(_) => self.notice_pinned,
            _ => false,
        };

        let pos = if self.visible {
            self.notice_pos(ctx)
        } else {
            pos2(ui::OFFSCREEN, ui::OFFSCREEN)
        };
        let action = if let Ok(mut d) = self.notice.lock() {
            if let Some(v) = view {
                d.view = v;
            }
            d.dark = self.dark;
            let was = d.open;
            d.open = show;
            if show && !was {
                d.first = true;
            }
            d.pos = pos;
            let a = d.action.take();
            a
        } else {
            None
        };
        if show {
            ctx.request_repaint_of(notice::viewport_id());
        }

        // 设置窗口里的版本信息（命令取出来再处理，别在锁里调 self 的方法）
        let mut cmd = None;
        let mut repaint_settings = false;
        if let Ok(mut d) = self.draft.lock() {
            let next = up_info(&st);
            if next != d.up {
                d.up = next;
                repaint_settings = true;
            }
            // macOS 没授权「自动化」时，跟随显示/隐藏其实不生效，设置里如实说
            let blocked = !platform::frontmost_available();
            if blocked != d.no_automation {
                d.no_automation = blocked;
                repaint_settings = true;
            }
            cmd = d.up_cmd.take();
        }
        if repaint_settings && self.settings_open {
            ctx.request_repaint_of(settings::viewport_id());
        }
        match cmd {
            Some(settings::UpdateCmd::Check) => {
                self.notice_dismissed = false;
                self.updater.check(ctx.clone());
            }
            Some(settings::UpdateCmd::Download) => {
                self.notice_dismissed = false;
                self.notice_pinned = true;
                if let update::Status::Found(rel) = st.clone() {
                    self.updater.download(rel, ctx.clone());
                }
            }
            Some(settings::UpdateCmd::Restart) => self.restart_with_update(ctx),
            Some(settings::UpdateCmd::OpenRelease) => open_release_page(),
            None => {}
        }

        match action {
            Some(notice::Action::Download) => {
                self.notice_pinned = true;
                if let update::Status::Found(rel) = st {
                    self.updater.download(rel, ctx.clone());
                }
            }
            Some(notice::Action::Retry) => {
                self.notice_pinned = true;
                self.notice_dismissed = false;
                self.updater.check(ctx.clone());
            }
            Some(notice::Action::Restart) => self.restart_with_update(ctx),
            Some(notice::Action::Dismiss) => self.notice_dismissed = true,
            None => {}
        }
    }

    /// 提示卡贴在 bar 上方；上面放不下就放下方
    fn notice_pos(&self, ctx: &egui::Context) -> Pos2 {
        let mon = self.monitor_rect(ctx);
        let bar = self.win.rect();
        let gap = NOTICE_GAP;
        let mut y = bar.top() - gap - notice::H;
        if y < mon.top() + 8.0 {
            y = (bar.bottom() + gap).min((mon.bottom() - notice::H - 8.0).max(mon.top() + 8.0));
        }
        let x = (bar.center().x - notice::W * 0.5).clamp(
            mon.left() + 8.0,
            (mon.right() - notice::W - 8.0).max(mon.left() + 8.0),
        );
        pos2(x.round(), y.round())
    }

    /// 保存现场 → 让"小工"进程换 exe → 自己退出
    fn restart_with_update(&mut self, ctx: &egui::Context) {
        self.cfg.save();
        self.tracker.save_cache();
        if self.updater.apply(true) {
            self.quit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// update::Status → 设置窗口那一行要显示的东西
fn up_info(st: &update::Status) -> settings::UpInfo {
    let line1 = format!("Tokenbar {} · 打包于 {}", ui::version(), ui::built_at());
    let mut info = settings::UpInfo {
        line1,
        ..Default::default()
    };
    match st {
        update::Status::Idle => {}
        update::Status::Checking => {
            info.status = "正在检测…".into();
            info.button = "检测中";
            info.busy = true;
        }
        update::Status::Latest => {
            info.status = "已经是最新版本".into();
        }
        update::Status::Found(rel) => {
            info.status = format!("发现新版本 v{}", rel.version);
            info.button = "更新";
            info.primary = true;
            info.cmd = settings::UpdateCmd::Download;
        }
        update::Status::Downloading { got, total } => {
            let pct = total
                .filter(|t| *t > 0)
                .map(|t| (*got as f32 / t as f32 * 100.0).clamp(0.0, 100.0));
            info.status = match pct {
                Some(p) => format!("正在下载 {p:.0}%"),
                None => "正在下载…".into(),
            };
            info.button = "下载中";
            info.busy = true;
            info.progress = Some(pct.map(|p| p / 100.0).unwrap_or(0.0));
        }
        update::Status::Ready { version } => {
            info.status = format!("v{version} 已下载，重启后生效");
            info.button = "立即重启";
            info.primary = true;
            info.cmd = settings::UpdateCmd::Restart;
        }
        update::Status::Manual(newer) => match newer {
            // 查到比当前新的版本：macOS 不就地替换 .app，引导去 Release 页下载
            Some(v) => {
                info.status = format!("发现新版本 v{v}");
                info.button = "去下载";
                info.primary = true;
                info.cmd = settings::UpdateCmd::OpenRelease;
            }
            None => {
                info.status = "已经是最新版本".into();
            }
        },
        update::Status::Failed(msg) => {
            info.status = format!("更新失败：{msg}");
            info.button = "重试";
            info.primary = true;
            info.cmd = settings::UpdateCmd::Check;
        }
    }
    info
}

/// 用系统浏览器打开发布页（macOS 没有应用内更新，给用户一条能点的路）
fn open_release_page() {
    let url = format!("https://gitee.com/{}/releases", update::REPO);
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(windows)]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", &url])
        .spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
}

fn close_settings(d: &mut settings::Draft, open: &mut bool) {
    d.open = false;
    d.result = None;
    *open = false;
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.quit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.polished {
            if let Ok(h) = frame.window_handle() {
                platform::polish_window(h.as_raw());
                self.polished = true;
            }
        }
        let ctx = ui.ctx().clone();
        if self.hwnd.is_none() {
            if let Ok(h) = frame.window_handle() {
                let raw = h.as_raw();
                self.hwnd = platform::hwnd_of(&raw);
            }
        }
        self.tick(&ctx);
        self.draw(&ctx);
        if self.settings_open {
            settings::show(&ctx, self.draft.clone());
        }
        notice::show(&ctx, self.notice.clone());
        // 第一次运行：等这一帧的位置指令都发完了，再用系统坐标把它挪到 WorkBuddy 那块屏幕
        self.place_on_screen();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

fn wake_file() -> Option<std::path::PathBuf> {
    // 跟配置放一起（exe 可能在不给写的目录里）；两个实例算出来的是同一个路径
    Some(config::config_dir().join("tokenbar.wake"))
}

fn wake_requested() -> bool {
    if let Some(p) = wake_file() {
        if p.exists() {
            let _ = std::fs::remove_file(&p);
            return true;
        }
    }
    false
}

fn system_dark(ctx: &egui::Context) -> bool {
    match ctx.system_theme() {
        Some(egui::Theme::Dark) => true,
        Some(egui::Theme::Light) => false,
        _ => true,
    }
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    #[cfg(windows)]
    let candidates: [(&str, &str, u32); 5] = [
        ("ui-cjk", "C:/Windows/Fonts/msyh.ttc", 0),
        ("ui-cjk-alt", "C:/Windows/Fonts/Deng.ttf", 0),
        ("ui-sb", "C:/Windows/Fonts/seguisb.ttf", 0),
        ("ui-sb-alt", "C:/Windows/Fonts/segoeuib.ttf", 0),
        ("ui-sb-cjk", "C:/Windows/Fonts/msyhbd.ttc", 0),
    ];
    #[cfg(target_os = "macos")]
    let candidates: [(&str, &str, u32); 4] = [
        ("ui-cjk", "/System/Library/Fonts/PingFang.ttc", 0),
        (
            "ui-cjk-alt",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            0,
        ),
        ("ui-sb", "/System/Library/Fonts/SFNS.ttf", 0),
        ("ui-sb-alt", "/System/Library/Fonts/Helvetica.ttc", 0),
    ];
    #[cfg(not(any(windows, target_os = "macos")))]
    let candidates: [(&str, &str, u32); 4] = [
        (
            "ui-cjk",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            0,
        ),
        (
            "ui-cjk-alt",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            0,
        ),
        (
            "ui-sb",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            0,
        ),
        (
            "ui-sb-alt",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            0,
        ),
    ];

    for (name, path, index) in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fd = egui::FontData::from_owned(bytes);
            fd.index = index;
            fonts.font_data.insert(name.to_owned(), Arc::new(fd));
        }
    }

    let pick = |list: &[&str]| -> Vec<String> {
        let v: Vec<String> = list
            .iter()
            .filter(|n| fonts.font_data.contains_key(**n))
            .map(|n| n.to_string())
            .collect();
        if v.is_empty() {
            vec!["Ubuntu-Light".to_string()]
        } else {
            v
        }
    };
    let regular = pick(&["ui-cjk", "ui-cjk-alt", "ui-sb", "ui-sb-alt"]);
    // 加粗族：拉丁/数字用 Segoe 半粗，中文回退到雅黑 Bold
    let strong = pick(&["ui-sb", "ui-sb-alt", "ui-sb-cjk", "ui-cjk", "ui-cjk-alt"]);

    fonts
        .families
        .insert(egui::FontFamily::Name("regular".into()), regular.clone());
    fonts
        .families
        .insert(egui::FontFamily::Name("sb".into()), strong.clone());
    fonts
        .families
        .insert(egui::FontFamily::Proportional, regular);
    fonts.families.insert(egui::FontFamily::Monospace, strong);

    ctx.set_fonts(fonts);
}

fn main() {
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let has = |name: &str| argv.iter().any(|a| a.to_string_lossy() == name);

    // 只报版本号（脚本、更新器用）
    if has("--version") {
        println!("{}", update::VERSION);
        return;
    }
    // 更新器的小工进程：等旧进程退出 → 换 exe → 需要的话再拉起来
    if let Some(i) = argv.iter().position(|a| a.to_string_lossy() == "--apply-update") {
        update::run_applier(&argv[i + 1..]);
    }
    if has("--dump") {
        let mut t = Tracker::new();
        t.poll();
        let s = t.snapshot();
        println!(
            "{{\"status\":\"{:?}\",\"active\":{},\"conv\":\"{}\",\"model\":\"{}\",\"turns\":{},\"elapsed_ms\":{:.0},\"turn\":{},\"total\":{}}}",
            s.status,
            s.active,
            s.current_conv,
            s.model,
            s.turns_count,
            s.elapsed_ms,
            data::stats_json_compact(s.turn.as_ref()),
            data::stats_json_compact(s.total.as_ref())
        );
        return;
    }
    // 同步查一次更新，结果打成 JSON（自检脚本用）
    if has("--check-update") {
        println!("{}", update::check_once_json());
        return;
    }
    if !platform::single_instance() {
        // 已有实例在跑：留一个唤醒标记，让它自己现身
        if let Some(p) = wake_file() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&p, b"1");
        }
        return;
    }
    let cfg_path = config::config_path();
    let cfg = Config::new(cfg_path);

    let viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_inner_size([440.0, ui::BAR_H])
        .with_taskbar(false)
        .with_always_on_top()
        .with_has_shadow(false)
        .with_close_button(false);

    let options = eframe::NativeOptions {
        viewport,
        centered: false,
        ..Default::default()
    };

    let _ = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, cfg)))),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::Status;

    /// 设置窗口「版本」那一行：macOS 查到新版时引导去 Release 页，而不是给一个假的下载按钮
    #[test]
    fn manual_status_points_to_release_page() {
        let info = up_info(&Status::Manual(Some("9.9.9".into())));
        assert_eq!(info.status, "发现新版本 v9.9.9");
        assert_eq!(info.button, "去下载");
        assert!(info.primary, "有新版时按钮要用主色，让人看得见");
        assert!(!info.busy, "按钮必须能点");
        assert_eq!(info.cmd, settings::UpdateCmd::OpenRelease);
    }

    /// 没查到更新的版本：还是老老实实显示「检测更新」，别把用户往发布页推
    #[test]
    fn manual_without_newer_stays_on_check() {
        let info = up_info(&Status::Manual(None));
        assert_eq!(info.status, "已经是最新版本");
        assert_eq!(info.button, "检测更新");
        assert_eq!(info.cmd, settings::UpdateCmd::Check);
    }

    /// 版本卡片里那两行字不能被按钮挤掉：按可用宽度实测一遍（这正是之前被裁的地方）
    #[test]
    fn version_row_text_fits_beside_button() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        // 让字体真正建起来（renderer 不需要，纯排版）；纹理增量要清掉，否则 drop 时报警
        let mut out = ctx.run_pass(Default::default(), |_| {});
        out.textures_delta.clear();

        // version_card 的排版：卡片宽 = 窗口宽 - 2*PAD；文字右边到按钮左边留 10
        let card_w = settings::WIN_W - 2.0 * 20.0;
        let text_max = card_w - 14.0 - 84.0 - 10.0 - 14.0;

        for st in [
            Status::Manual(Some("9.9.9".into())),
            Status::Manual(None),
            Status::Latest,
            Status::Checking,
        ] {
            let info = up_info(&st);
            let w = ui::text_w(&ctx, &info.status, 11.5, ui::FONT_SB);
            assert!(
                w <= text_max,
                "状态文案「{}」实测 {w:.0}px，超过可用 {text_max:.0}px，会被按钮裁掉",
                info.status
            );
            let bw = ui::text_w(&ctx, info.button, 11.0, ui::FONT_R);
            assert!(bw <= 84.0 - 8.0, "按钮文案「{}」太宽（{bw:.0}px）", info.button);
        }

        // 第一行（版本号 + 打包时间）也不能被裁
        let info = up_info(&Status::Latest);
        let w = ui::text_w(&ctx, &info.line1, 10.0, ui::FONT_R);
        assert!(
            w <= text_max,
            "版本行「{}」实测 {w:.0}px，超过 {text_max:.0}px",
            info.line1
        );
    }

    /// 检查中/已是最新这两个状态，按钮仍然是「检测更新」
    #[test]
    fn latest_and_checking_keep_check_button() {
        let latest = up_info(&Status::Latest);
        assert_eq!(latest.status, "已经是最新版本");
        assert_eq!(latest.button, "检测更新");
        assert_eq!(latest.cmd, settings::UpdateCmd::Check);

        let checking = up_info(&Status::Checking);
        assert_eq!(checking.button, "检测中");
        assert!(checking.busy);
    }
}
