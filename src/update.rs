//! 自动更新：跟 Gitee 上最新 release 比版本号，下载新的 exe，退出后在后台替换。
//!
//! 三条硬规矩：
//! - 只在 Windows 上做（release 里只有 win64 的 exe）。
//! - 下载下来先验一遍（是不是 exe、版本号对不对），验不过就删掉，绝不留半成品。
//! - 运行中的 exe 不能被自己覆盖：先把 exe 的副本放到临时目录当"小工"，
//!   由它等本进程退出、再把新文件换上去（要重开就直接拉起来）。

use serde_json::Value;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 本程序的版本号（唯一来源：Cargo.toml）
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// 打包时间（build.rs 编译时写进 exe）
pub const BUILT_AT: &str = env!("TOKENBAR_BUILT_AT");
pub const REPO: &str = "mrpu2020/tokenbar-workbuddy";

/// 下载好的新 exe 先放这里，退出时才换上去
const NEW_EXE: &str = "Tokenbar.exe.new";
/// 记录"已经下载好但还没换"的版本，重启后还能接着提示
const PENDING_JSON: &str = "tokenbar-update.json";

/// 云端的一个发行版
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
    #[allow(dead_code)]
    pub notes: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// 还没查过
    Idle,
    Checking,
    /// 已经是最新
    Latest,
    /// 发现新版本，等用户点
    Found(Release),
    Downloading {
        got: u64,
        total: Option<u64>,
    },
    /// 下载并校验完，等重启（或等本次退出时自动换）
    Ready {
        version: String,
    },
    /// 这个平台不做应用内更新（macOS 的 .app 不就地替换）。
    /// `Some(版本)` = 查到了比当前新的版本，引导用户去 Release 页下载。
    Manual(Option<String>),
    Failed(String),
}

impl Status {
    pub fn busy(&self) -> bool {
        matches!(self, Status::Checking | Status::Downloading { .. })
    }
    #[allow(dead_code)]
    pub fn is_manual(&self) -> bool {
        matches!(self, Status::Manual(_))
    }
    pub fn is_ready(&self) -> bool {
        matches!(self, Status::Ready { .. })
    }
}

/// 已下载待替换的新版本
#[derive(Clone, Debug)]
pub struct Pending {
    pub version: String,
    pub file: PathBuf,
}

/// 跟界面共享的更新状态（后台线程写，界面每帧读）
pub struct Updater {
    pub status: Arc<Mutex<Status>>,
    lock: Arc<AtomicBool>,
    pub pending: Arc<Mutex<Option<Pending>>>,
    dir: PathBuf,
}

impl Updater {
    pub fn new() -> Self {
        let dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let status = Arc::new(Mutex::new(Status::Idle));
        let pending = Arc::new(Mutex::new(None));

        // 只有 new 文件、没有记录（或反过来）：都是半成品，清掉
        let new_exe = dir.join(NEW_EXE);
        match (read_pending(&dir), new_exe.is_file()) {
            (None, true) => {
                let _ = fs::remove_file(&new_exe);
            }
            (Some(_), false) => {
                let _ = fs::remove_file(dir.join(PENDING_JSON));
            }
            _ => {}
        }

        // 上次已经下好了但没换（或者换了没重启）：接着提示
        if let Some(ver) = read_pending(&dir) {
            let file = dir.join(NEW_EXE);
            if file.is_file() {
                *pending.lock().unwrap() = Some(Pending {
                    version: ver.clone(),
                    file,
                });
                *status.lock().unwrap() = Status::Ready { version: ver };
            }
        }

        Self {
            status,
            lock: Arc::new(AtomicBool::new(false)),
            pending,
            dir,
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or(Status::Idle)
    }

    pub fn pending_version(&self) -> Option<String> {
        self.pending
            .lock()
            .ok()
            .and_then(|p| p.as_ref().map(|p| p.version.clone()))
    }

    /// 挪到后台线程去查；`ctx` 用来让界面立刻刷新
    pub fn check(&self, ctx: egui::Context) {
        if self.lock.swap(true, Ordering::SeqCst) {
            return;
        }
        set(&self.status, Status::Checking);
        let status = self.status.clone();
        let lock = self.lock.clone();
        std::thread::spawn(move || {
            let next = match fetch_release() {
                Ok(rel) => {
                    if !is_newer(&rel.version, VERSION) {
                        Status::Latest
                    } else if !cfg!(windows) {
                        // macOS 的包是一个 .app，应用内替换风险太大：
                        // 只告诉用户有新版本，让他去 Release 页下 dmg
                        Status::Manual(Some(rel.version))
                    } else if rel.url.is_empty() {
                        Status::Failed(format!("新版本 v{} 没有 exe 附件", rel.version))
                    } else {
                        Status::Found(rel)
                    }
                }
                Err(e) => Status::Failed(e),
            };
            set(&status, next);
            lock.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// 下载 + 校验，完成后进入 Ready
    pub fn download(&self, rel: Release, ctx: egui::Context) {
        if self.lock.swap(true, Ordering::SeqCst) {
            return;
        }
        let status = self.status.clone();
        let lock = self.lock.clone();
        let pending = self.pending.clone();
        let dir = self.dir.clone();
        set(
            &status,
            Status::Downloading {
                got: 0,
                total: None,
            },
        );
        std::thread::spawn(move || {
            let dst = dir.join(NEW_EXE);
            let _ = fs::remove_file(&dst);
            let mut last = Instant::now();
            let res = http_download(&rel.url, &dst, |got, total| {
                set(&status, Status::Downloading { got, total });
                if last.elapsed() > Duration::from_millis(120) {
                    last = Instant::now();
                    ctx.request_repaint();
                }
            })
            .and_then(|_| verify_exe(&dst, &rel.version).map_err(|e| {
                let _ = fs::remove_file(&dst);
                e
            }));

            let next = match res {
                Ok(()) => {
                    write_pending(&dir, &rel.version);
                    *pending.lock().unwrap() = Some(Pending {
                        version: rel.version.clone(),
                        file: dst,
                    });
                    Status::Ready {
                        version: rel.version.clone(),
                    }
                }
                Err(e) => Status::Failed(e),
            };
            set(&status, next);
            lock.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// 拉起"小工"进程去替换 exe（调用方随后应立即退出）
    pub fn apply(&self, restart: bool) -> bool {
        let file = match self.pending.lock().ok().and_then(|p| p.clone()) {
            Some(p) => p.file,
            None => return false,
        };
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let worker = std::env::temp_dir().join(format!(
            "Tokenbar-update-{}-{}.exe",
            std::process::id(),
            now_tag()
        ));
        if fs::copy(&exe, &worker).is_err() {
            return false;
        }
        let mut cmd = Command::new(&worker);
        cmd.arg("--apply-update")
            .arg(std::process::id().to_string())
            .arg(&exe)
            .arg(&file)
            .arg(if restart { "1" } else { "0" })
            .current_dir(std::env::temp_dir());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.spawn().is_ok()
    }
}

impl Default for Updater {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- 替换进程

/// `--apply-update <pid> <目标 exe> <新文件> <是否重开>`
pub fn run_applier(args: &[std::ffi::OsString]) -> ! {
    let pid: u32 = args
        .first()
        .map(|s| s.to_string_lossy().trim().parse().unwrap_or(0))
        .unwrap_or(0);
    let target = args.get(1).map(PathBuf::from);
    let new = args.get(2).map(PathBuf::from);
    let restart = args
        .get(3)
        .map(|s| s.to_string_lossy() == "1")
        .unwrap_or(false);
    let (Some(target), Some(new)) = (target, new) else {
        std::process::exit(2);
    };

    wait_exit(pid, Duration::from_secs(30));
    // 旧进程刚退出时文件句柄可能还没放开，重试到成功为止
    let mut ok = false;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if let Some(dir) = target.parent() {
            if fs::metadata(dir).is_err() {
                break;
            }
        }
        if fs::rename(&new, &target).is_ok() {
            ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    if ok {
        if let Some(dir) = target.parent() {
            let _ = fs::remove_file(dir.join(PENDING_JSON));
        }
        if restart {
            let mut cmd = Command::new(&target);
            if let Some(dir) = target.parent() {
                cmd.current_dir(dir);
            }
            let _ = cmd.spawn();
        }
    }
    std::process::exit(if ok { 0 } else { 1 })
}

#[cfg(windows)]
fn wait_exit(pid: u32, timeout: Duration) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};
    const SYNCHRONIZE: u32 = 0x0010_0000;
    if pid == 0 {
        return;
    }
    unsafe {
        let h = OpenProcess(SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return;
        }
        WaitForSingleObject(h, timeout.as_millis() as u32);
        CloseHandle(h);
    }
}

#[cfg(not(windows))]
fn wait_exit(pid: u32, timeout: Duration) {
    let started = Instant::now();
    while started.elapsed() < timeout {
        let alive = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !alive {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ---------------------------------------------------------------- 版本号

/// "v1.5.0" → [1, 5, 0]；后面带的 -beta 之类忽略
pub fn parse_version(s: &str) -> Vec<u32> {
    let s = s.trim();
    let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().unwrap_or(0))
        .collect()
}

/// candidate 比 current 新吗（1.5 > 1.4.9；1.5.0 == 1.5）
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let a = parse_version(candidate);
    let b = parse_version(current);
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

// ---------------------------------------------------------------- 云端

/// 解析 release JSON：认 tag_name 当版本，只认 .exe 附件（源码 zip/tar.gz 直接跳过）
pub fn parse_release(json: &str) -> Result<Release, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("云端返回看不懂：{e}"))?;
    let tag = v
        .get("tag_name")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("name").and_then(|x| x.as_str()))
        .unwrap_or("")
        .trim()
        .to_string();
    let version = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(&tag)
        .to_string();
    if version.is_empty() {
        return Err("云端 release 没有版本号".into());
    }
    let notes = v
        .get("body")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    let mut best: Option<(bool, String)> = None;
    if let Some(list) = v.get("assets").and_then(|x| x.as_array()) {
        for a in list {
            let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let url = a
                .get("browser_download_url")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let lower = name.to_ascii_lowercase();
            if url.is_empty() || !lower.ends_with(".exe") {
                continue;
            }
            let exact = lower == "tokenbar.exe";
            let better = match &best {
                None => true,
                Some((e, _)) => exact && !*e,
            };
            if better {
                best = Some((exact, url.to_string()));
            }
        }
    }
    // 老版本 release 里可能只有源码 zip：不在这里报错，交给上层按版本号决定要不要提醒
    let url = best.map(|(_, u)| u).unwrap_or_default();
    Ok(Release {
        version,
        url,
        notes,
    })
}

fn api_url() -> String {
    std::env::var("TOKENBAR_GITEE_API").unwrap_or_else(|_| latest_api())
}

/// 云端最新发行版的地址（公开仓库，不需要令牌）
pub fn latest_api() -> String {
    format!("https://gitee.com/api/v5/repos/{REPO}/releases/latest")
}

/// 同步查一次，打成一行 JSON（`--check-update`，给自检脚本看）
pub fn check_once_json() -> String {
    match fetch_release() {
        Ok(rel) => serde_json::json!({
            "status": "ok",
            "current": VERSION,
            "version": rel.version,
            "newer": is_newer(&rel.version, VERSION),
            "url": rel.url,
        })
        .to_string(),
        Err(e) => serde_json::json!({
            "status": "error",
            "current": VERSION,
            "message": e,
        })
        .to_string(),
    }
}

fn fetch_release() -> Result<Release, String> {
    let body = http_get(&api_url())?;
    parse_release(&body)
}

fn agent() -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(20))
        .timeout_write(Duration::from_secs(20))
        .user_agent(&format!("Tokenbar/{VERSION}"));
    // ureq 2 不会自己挑 TLS 后端：走系统的（Windows=schannel，macOS=Security）
    if let Ok(tls) = ureq::native_tls::TlsConnector::new() {
        b = b.tls_connector(std::sync::Arc::new(tls));
    }
    b.build()
}

fn http_get(url: &str) -> Result<String, String> {
    match agent().get(url).call() {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| format!("读取云端数据失败：{e}")),
        Err(ureq::Error::Status(404, _)) => Err("云端还没有发布版本".into()),
        Err(ureq::Error::Status(code, _)) => Err(format!("云端返回 {code}")),
        Err(e) => Err(format!("连不上 Gitee：{e}")),
    }
}

fn http_download(
    url: &str,
    dst: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let resp = match agent().get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return Err(format!("下载失败：HTTP {code}")),
        Err(e) => return Err(format!("下载失败：{e}")),
    };
    let total = resp
        .header("Content-Length")
        .and_then(|s| s.trim().parse::<u64>().ok());
    let mut reader = resp.into_reader();
    let mut file = File::create(dst).map_err(|e| format!("写不进文件：{e}"))?;
    let mut buf = [0u8; 64 * 1024];
    let mut got = 0u64;
    progress(0, total);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("下载中断：{e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("写不进文件：{e}"))?;
        got += n as u64;
        progress(got, total);
    }
    file.flush().map_err(|e| format!("写不进文件：{e}"))?;
    if let Some(t) = total {
        if got != t {
            return Err(format!("下载不完整（{got}/{t} 字节）"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 校验与落盘

/// 是不是一个像样的 exe，而且是我们要的那个版本
fn verify_exe(path: &Path, version: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("读取下载文件失败：{e}"))?;
    if bytes.len() < 120_000 {
        return Err("下载下来的文件太小，不像程序".into());
    }
    if &bytes[0..2] != b"MZ" {
        return Err("下载下来的不是 Windows 程序".into());
    }
    let off = u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]]) as usize;
    if off + 4 > bytes.len() || &bytes[off..off + 4] != b"PE\0\0" {
        return Err("下载下来的不是 Windows 程序".into());
    }
    // VERSIONINFO 里的版本串是 UTF-16LE，能搜到才说明下到的是这个版本
    let needle: Vec<u8> = version
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    if !needle.is_empty() && !bytes.windows(needle.len()).any(|w| w == needle) {
        return Err(format!("下载的版本跟云端标的 {version} 不一致"));
    }
    Ok(())
}

fn read_pending(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join(PENDING_JSON)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let ver = v.get("version")?.as_str()?.trim().to_string();
    if ver.is_empty() {
        None
    } else {
        Some(ver)
    }
}

fn write_pending(dir: &Path, version: &str) {
    let body = format!("{{\"version\":\"{version}\",\"file\":\"{NEW_EXE}\"}}");
    let _ = fs::write(dir.join(PENDING_JSON), body);
}

fn set(slot: &Arc<Mutex<Status>>, s: Status) {
    if let Ok(mut g) = slot.lock() {
        *g = s;
    }
}

fn now_tag() -> String {
    chrono::Local::now().format("%H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("1.5.0", "1.4.1"));
        assert!(is_newer("v1.5.0", "1.4.9"));
        assert!(is_newer("1.5", "1.4.1"));
        assert!(is_newer("1.10.0", "1.9.9"), "10 > 9 要按数字比");
        assert!(!is_newer("1.4.1", "1.4.1"));
        assert!(!is_newer("1.4.0", "1.4.1"));
        assert!(!is_newer("1.5.0", "1.5.0.0"), "补零不算新版本");
        assert!(!is_newer("v1.4", "1.4.0"));
    }

    #[test]
    fn parses_versions_with_noise() {
        assert_eq!(parse_version("v1.5.0"), vec![1, 5, 0]);
        assert_eq!(parse_version("1.5.0-beta.2"), vec![1, 5, 0, 2]);
        assert_eq!(parse_version("  V2 "), vec![2]);
        assert_eq!(parse_version("latest"), Vec::<u32>::new());
    }

    #[test]
    fn picks_exe_asset_only() {
        // 真实 Gitee 返回里混着源码 zip / tar.gz
        let json = r#"{
            "tag_name": "v1.5.0",
            "name": "v1.5.0",
            "body": "修了几个问题",
            "assets": [
                {"name": "v1.5.0.zip", "browser_download_url": "https://gitee.com/a.zip"},
                {"name": "v1.5.0.tar.gz", "browser_download_url": "https://gitee.com/a.tar.gz"},
                {"name": "Tokenbar.exe", "browser_download_url": "https://gitee.com/x/Tokenbar.exe"}
            ]
        }"#;
        let rel = parse_release(json).unwrap();
        assert_eq!(rel.version, "1.5.0");
        assert_eq!(rel.url, "https://gitee.com/x/Tokenbar.exe");
        assert_eq!(rel.notes, "修了几个问题");
    }

    #[test]
    fn prefers_exact_name_among_many_exes() {
        let json = r#"{"tag_name":"1.6.0","assets":[
            {"name":"Tokenbar-1.6.0.exe","browser_download_url":"https://g/a.exe"},
            {"name":"Tokenbar.exe","browser_download_url":"https://g/b.exe"}
        ]}"#;
        assert_eq!(parse_release(json).unwrap().url, "https://g/b.exe");
    }

    #[test]
    fn tolerates_release_without_exe() {
        // 老 release 只有源码包：解析得出来，但 url 为空，上层据此不提示
        let json = r#"{"tag_name":"v1.5.0","assets":[{"name":"a.zip","browser_download_url":"https://g/a.zip"}]}"#;
        let rel = parse_release(json).unwrap();
        assert_eq!(rel.version, "1.5.0");
        assert!(rel.url.is_empty());
        // 不是 JSON（比如网关返回的 HTML）必须报错
        assert!(parse_release("<html>502</html>").is_err());
    }

    /// 真要联网的手工自检：真从 Gitee 把最新版下载下来验一遍。
    ///   cargo test real_release -- --ignored --nocapture
    #[test]
    #[ignore = "需要联网"]
    fn real_release_is_downloadable() {
        let rel = fetch_release().expect("查不到最新 release");
        println!("线上最新版本：v{}", rel.version);
        assert!(!rel.url.is_empty(), "最新 release 里没有 exe 附件");
        let dst = std::env::temp_dir().join("tokenbar_real_download.exe");
        let _ = fs::remove_file(&dst);
        http_download(&rel.url, &dst, |got, total| {
            if got % (1024 * 1024) < 65536 {
                println!("  {got} / {:?}", total);
            }
        })
        .expect("下载失败（302 跳转 / 证书 / 网络）");
        let size = fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
        println!("下载完成：{size} 字节");
        assert!(size > 1_000_000);
        verify_exe(&dst, &rel.version).expect("下载的文件校验不过");
        let _ = fs::remove_file(&dst);
    }

    #[test]
    fn current_version_is_sane() {
        assert!(!VERSION.is_empty());
        assert!(!is_newer(VERSION, VERSION));
        assert!(VERSION.starts_with(char::is_numeric), "{VERSION}");
        assert!(BUILT_AT.len() >= 10, "{BUILT_AT}");
    }
}
