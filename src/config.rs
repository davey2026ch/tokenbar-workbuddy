//! INI 配置：位置、主题、指标顺序与开关，全部实时落盘。

use crate::metric::Metric;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutMode {
    Horizontal,
    Vertical,
}

impl LayoutMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LayoutMode::Horizontal => "horizontal",
            LayoutMode::Vertical => "vertical",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "vertical" | "v" | "竖排" => LayoutMode::Vertical,
            _ => LayoutMode::Horizontal,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            LayoutMode::Horizontal => "横排（两行）",
            LayoutMode::Vertical => "竖排（两列）",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemePref {
    System,
    Dark,
    Light,
}

impl ThemePref {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemePref::System => "system",
            ThemePref::Dark => "dark",
            ThemePref::Light => "light",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" => ThemePref::Dark,
            "light" => ThemePref::Light,
            _ => ThemePref::System,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ThemePref::System => "跟随系统",
            ThemePref::Dark => "深色",
            ThemePref::Light => "浅色",
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub path: PathBuf,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub theme: ThemePref,
    pub layout: LayoutMode,
    pub follow: bool,
    /// 指标顺序（全部 6 项，含被隐藏的）
    pub order: Vec<Metric>,
    /// 每一项是否显示，下标见 `Metric::index`
    enabled: [bool; Metric::ALL.len()],
    last_written: String,
}

impl Config {
    pub fn default_order() -> Vec<Metric> {
        Metric::ALL.to_vec()
    }

    /// 默认显示除「缓存」外的五项：本地模型大多不上报缓存，留着一列「—」反而碍眼
    fn default_enabled() -> [bool; Metric::ALL.len()] {
        let mut on = [true; Metric::ALL.len()];
        on[Metric::Cache.index()] = false;
        on
    }

    pub fn new(path: PathBuf) -> Self {
        let mut cfg = Self {
            path: path.clone(),
            x: None,
            y: None,
            theme: ThemePref::System,
            layout: LayoutMode::Horizontal,
            follow: true,
            order: Self::default_order(),
            enabled: Self::default_enabled(),
            last_written: String::new(),
        };
        if let Ok(text) = fs::read_to_string(&path) {
            cfg.parse(&text);
        }
        cfg
    }

    fn parse(&mut self, text: &str) {
        let mut section = String::new();
        let mut order_seen = false;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_ascii_lowercase();
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            match section.as_str() {
                "window" => match key.as_str() {
                    "x" => self.x = val.parse::<f32>().ok(),
                    "y" => self.y = val.parse::<f32>().ok(),
                    _ => {}
                },
                "general" => match key.as_str() {
                    "theme" => self.theme = ThemePref::parse(val),
                    "layout" => self.layout = LayoutMode::parse(val),
                    "follow_workbuddy" => self.follow = parse_bool(val, true),
                    _ => {}
                },
                "metrics" => {
                    if key == "order" {
                        self.order = parse_order(val);
                        order_seen = true;
                    } else if let Some(m) = Metric::from_key(&key) {
                        self.enabled[m.index()] = parse_bool(val, true);
                    }
                }
                _ => {}
            }
        }
        if !order_seen {
            self.order = Self::default_order();
        }
        // 至少保证有一项可见，否则用户会看到空 bar 却不知道原因
        if self.visible_metrics().is_empty() {
            self.enabled = [true; Metric::ALL.len()];
        }
    }

    pub fn visible_metrics(&self) -> Vec<Metric> {
        self.order
            .iter()
            .copied()
            .filter(|m| self.is_enabled(*m))
            .collect()
    }

    pub fn set_enabled(&mut self, key: Metric, on: bool) {
        self.enabled[key.index()] = on;
    }

    pub fn is_enabled(&self, key: Metric) -> bool {
        self.enabled[key.index()]
    }

    pub fn set_position(&mut self, x: f32, y: f32) {
        self.x = Some(x.round());
        self.y = Some(y.round());
    }

    /// 只把"外观/行为/指标"这类偏好抄过来，位置保持调用方当前值。
    /// 设置窗口里的草稿生效走这里，避免拖动位置被草稿里的旧坐标覆盖。
    pub fn apply_prefs_from(&mut self, other: &Config) {
        self.theme = other.theme;
        self.layout = other.layout;
        self.follow = other.follow;
        self.order = other.order.clone();
        self.enabled = other.enabled;
    }

    fn serialize(&self) -> String {
        let mut s = String::new();
        s.push_str("; Tokenbar 配置文件（首次运行自动生成）\n");
        s.push_str("; 改完保存即可，程序运行时界面上的改动也会实时写回。\n");
        s.push_str("; 末尾 [meta] 记着写入它的程序版本与打包时间，方便对照是哪个版本。\n\n");
        s.push_str("[window]\n");
        if let (Some(x), Some(y)) = (self.x, self.y) {
            s.push_str(&format!("x={}\n", x.round() as i64));
            s.push_str(&format!("y={}\n", y.round() as i64));
        }
        s.push_str("\n[general]\n");
        s.push_str(&format!("theme={}\n", self.theme.as_str()));
        s.push_str(&format!("layout={}\n", self.layout.as_str()));
        s.push_str(&format!(
            "follow_workbuddy={}\n",
            if self.follow { 1 } else { 0 }
        ));
        s.push_str("\n[metrics]\n");
        let order: Vec<&str> = self.order.iter().map(|m| m.key()).collect();
        s.push_str(&format!("order={}\n", order.join(",")));
        for m in Metric::ALL {
            let on = self.is_enabled(m);
            s.push_str(&format!("{}={}\n", m.key(), if on { 1 } else { 0 }));
        }
        // 谁写的这份 ini：版本号 + 这个 exe 的打包时间，出问题时一眼看出是哪一版
        s.push_str("\n[meta]\n");
        s.push_str("; 自动记录，不用手改：写入这份配置的程序版本与打包时间\n");
        s.push_str(&format!("version={}\n", crate::update::VERSION));
        s.push_str(&format!("packaged={}\n", crate::update::BUILT_AT));
        s
    }

    /// 有变化时才写盘，避免每帧抖动 IO。
    pub fn save(&mut self) {
        let text = self.serialize();
        if text == self.last_written {
            return;
        }
        let tmp = self.path.with_extension("ini.tmp");
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let body = text.replace('\n', "\r\n");
        if fs::write(&tmp, body.as_bytes()).is_ok() {
            // Windows 上目标文件被占用时 rename 会失败，退化为直接写
            if fs::rename(&tmp, &self.path).is_err() {
                let _ = fs::write(&self.path, body.as_bytes());
                let _ = fs::remove_file(&tmp);
            }
            self.last_written = text;
        }
    }
}

/// 顺序里出现未知/重复项时忽略，缺的补到末尾，保证永远是完整的 6 项
fn parse_order(val: &str) -> Vec<Metric> {
    let mut list: Vec<Metric> = Vec::new();
    for item in val.split(',') {
        if let Some(m) = Metric::from_key(item.trim()) {
            if !list.contains(&m) {
                list.push(m);
            }
        }
    }
    for m in Metric::ALL {
        if !list.contains(&m) {
            list.push(m);
        }
    }
    list
}

fn parse_bool(v: &str, default: bool) -> bool {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

/// 配置目录（ini 与统计快照都放这里）：
///
/// - Windows：`%APPDATA%\Tokenbar\`（即 `C:\Users\<你>\AppData\Roaming\Tokenbar`）
/// - macOS：`~/.tokenbar/`
/// - 其它：`$XDG_CONFIG_HOME/tokenbar` 或 `~/.config/tokenbar`
///
/// 测试与便携场景可以用环境变量 `TOKENBAR_CONFIG_DIR` 直接指定目录。
pub fn config_dir() -> PathBuf {
    if let Some(dir) = env_dir() {
        return dir;
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let dir = Path::new(&home).join(".tokenbar");
        migrate_from_app_support(&dir);
        return dir;
    }
    #[cfg(windows)]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        let dir = Path::new(&base).join("Tokenbar");
        migrate_from_exe_dir(&dir);
        return dir;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.config")))
            .unwrap_or_else(|| ".".into());
        Path::new(&base).join("tokenbar")
    }
}

/// 环境变量指定的目录（测试、绿色便携用），空字符串当作没设
fn env_dir() -> Option<PathBuf> {
    parse_dir(std::env::var("TOKENBAR_CONFIG_DIR").ok().as_deref())
}

fn parse_dir(v: Option<&str>) -> Option<PathBuf> {
    let v = v?.trim();
    if v.is_empty() {
        None
    } else {
        Some(PathBuf::from(v))
    }
}

/// 老版本把 ini / 快照写在 exe 同目录。换到用户目录后搬一次，别让设置白白丢掉。
#[cfg(windows)]
fn migrate_from_exe_dir(new_dir: &Path) {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(old) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        else {
            return;
        };
        migrate_files(&old, new_dir);
    });
}

/// 把老目录里的 ini / 快照拷到新目录（新目录已有同名文件就不动）
#[cfg_attr(not(windows), allow(dead_code))]
fn migrate_files(old: &Path, new_dir: &Path) {
    if old == new_dir {
        return;
    }
    for name in ["tokenbar.ini", "tokenbar-stats.json"] {
        let from = old.join(name);
        let to = new_dir.join(name);
        if from.is_file() && !to.exists() {
            let _ = fs::create_dir_all(new_dir);
            let _ = fs::copy(&from, &to);
        }
    }
}

/// 各任务的累计统计快照，和 ini 放同一个目录，便于"任务切回去还能看到数字"。
pub fn stats_cache_path() -> PathBuf {
    config_dir().join("tokenbar-stats.json")
}

/// 配置文件本身
pub fn config_path() -> PathBuf {
    config_dir().join("tokenbar.ini")
}

/// 早期 mac 版把配置放在 ~/Library/Application Support/Tokenbar/，
/// 换到 ~/.tokenbar 后把老文件搬过来，别让设置白白丢掉。
#[cfg(target_os = "macos")]
fn migrate_from_app_support(new_dir: &Path) {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        let old = Path::new(&home)
            .join("Library")
            .join("Application Support")
            .join("Tokenbar");
        if !old.is_dir() {
            return;
        }
        for name in ["tokenbar.ini", "tokenbar-stats.json"] {
            let from = old.join(name);
            let to = new_dir.join(name);
            if from.is_file() && !to.exists() {
                let _ = fs::create_dir_all(new_dir);
                let _ = fs::copy(&from, &to);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_ini(tag: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tokenbar_cfg_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokenbar.ini");
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn normalizes_order_and_flags() {
        let path = tmp_ini(
            "order",
            "[metrics]\norder=cache,unknown,ttft,cache\nttft=0\ncache=1\n",
        );
        let cfg = Config::new(path.clone());
        assert_eq!(
            cfg.order,
            vec![
                Metric::Cache,
                Metric::Ttft,
                Metric::Duration,
                Metric::Tps,
                Metric::Input,
                Metric::Output
            ],
            "未知项应丢弃、重复项应去重、缺的补到末尾"
        );
        assert!(!cfg.is_enabled(Metric::Ttft));
        assert_eq!(
            cfg.visible_metrics(),
            vec![
                Metric::Cache,
                Metric::Duration,
                Metric::Tps,
                Metric::Input,
                Metric::Output
            ]
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn cache_is_hidden_by_default() {
        // 没有 ini 时：默认只显示 5 项，缓存留空不占位
        let path = tmp_ini("defaults", "");
        let cfg = Config::new(path.clone());
        assert!(!cfg.is_enabled(Metric::Cache), "缓存默认应隐藏");
        assert_eq!(cfg.visible_metrics().len(), Metric::ALL.len() - 1);
        assert!(cfg.order.contains(&Metric::Cache), "顺序里仍保留缓存，勾选后能回来");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn keeps_at_least_one_metric_visible() {
        let path = tmp_ini(
            "alloff",
            "[metrics]\norder=ttft,duration,tps,input,output,cache\nttft=0\nduration=0\ntps=0\ninput=0\noutput=0\ncache=0\n",
        );
        let cfg = Config::new(path.clone());
        assert_eq!(cfg.visible_metrics().len(), Metric::ALL.len());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn env_dir_parsing() {
        assert_eq!(
            parse_dir(Some("  D:/tmp/cfg  ")),
            Some(PathBuf::from("D:/tmp/cfg"))
        );
        assert_eq!(parse_dir(Some("   ")), None, "空字符串等于没设");
        assert_eq!(parse_dir(None), None);
    }

    #[test]
    fn migrates_old_files_but_never_overwrites() {
        let base = std::env::temp_dir().join(format!("tokenbar_mig_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let old = base.join("exe");
        let new = base.join("user").join("Tokenbar");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("tokenbar.ini"), "[general]\ntheme=dark\n").unwrap();
        fs::write(old.join("tokenbar-stats.json"), "{}").unwrap();

        migrate_files(&old, &new);
        assert!(new.join("tokenbar.ini").is_file(), "ini 应该被搬过去");
        assert!(new.join("tokenbar-stats.json").is_file(), "快照也要搬");

        // 新目录已经有 ini：不能被老文件盖掉
        fs::write(new.join("tokenbar.ini"), "keep").unwrap();
        fs::write(old.join("tokenbar.ini"), "old").unwrap();
        migrate_files(&old, &new);
        assert_eq!(
            fs::read_to_string(new.join("tokenbar.ini")).unwrap(),
            "keep"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn round_trips_through_save() {
        let path = tmp_ini("save", "");
        let mut cfg = Config::new(path.clone());
        cfg.theme = ThemePref::Light;
        cfg.layout = LayoutMode::Vertical;
        cfg.follow = false;
        cfg.order = vec![Metric::Output, Metric::Input];
        cfg.set_enabled(Metric::Output, false);
        cfg.save();

        let back = Config::new(path.clone());
        assert_eq!(back.theme, ThemePref::Light);
        assert_eq!(back.layout, LayoutMode::Vertical);
        assert!(!back.follow);
        assert_eq!(back.order.len(), Metric::ALL.len());
        assert_eq!(back.order[0], Metric::Output);
        assert!(!back.is_enabled(Metric::Output));

        // ini 自动记下"哪个版本、什么时候打的包"
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[meta]"), "{text}");
        assert!(text.contains(&format!("version={}", crate::update::VERSION)));
        assert!(text.contains("packaged="));
        let stamp = text
            .lines()
            .find_map(|l| l.strip_prefix("packaged="))
            .unwrap_or("");
        assert!(stamp.len() >= 10, "打包时间看起来不对：{stamp}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
