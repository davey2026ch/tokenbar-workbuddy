//! 把 assets/icon.ico（以及版本信息）编进 exe，资源管理器和任务栏才会显示图标。
//!
//! 只用 Windows SDK 自带的 rc.exe，不引入额外构建依赖；其它平台直接跳过。

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-env-changed=TOKENBAR_BUILD_TIME");

    // 打包时间：ini 里要记一笔"这个 exe 是什么时候打出来的"。
    // 打包脚本会显式传 TOKENBAR_BUILD_TIME，平时编译就用当前时间。
    let stamp = std::env::var("TOKENBAR_BUILD_TIME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        });
    println!("cargo:rustc-env=TOKENBAR_BUILT_AT={stamp}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let ico = Path::new("assets/icon.ico");
    if !ico.exists() {
        println!("cargo:warning=缺少 assets/icon.ico，exe 将没有图标");
        return;
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=找不到 Windows SDK 的 rc.exe，exe 将没有图标");
        return;
    };

    // rc.exe 在工作目录里找图标，先拷到 OUT_DIR
    let local_ico = out_dir.join("icon.ico");
    if std::fs::copy(ico, &local_ico).is_err() {
        println!("cargo:warning=拷贝图标失败，exe 将没有图标");
        return;
    }

    let ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let nums: Vec<u32> = ver
        .split('.')
        .map(|s| s.parse::<u32>().unwrap_or(0))
        .collect();
    let quad = format!(
        "{},{},{},0",
        nums.first().copied().unwrap_or(0),
        nums.get(1).copied().unwrap_or(0),
        nums.get(2).copied().unwrap_or(0)
    );

    let rc_text = format!(
        r#"1 ICON "icon.ico"
1 VERSIONINFO
FILEVERSION {quad}
PRODUCTVERSION {quad}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "mrpu2020"
            VALUE "FileDescription", "Tokenbar - metrics bar for WorkBuddy"
            VALUE "FileVersion", "{ver}"
            VALUE "InternalName", "Tokenbar"
            VALUE "LegalCopyright", "MIT License"
            VALUE "OriginalFilename", "Tokenbar.exe"
            VALUE "ProductName", "Tokenbar"
            VALUE "ProductVersion", "{ver}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );
    let rc_path = out_dir.join("icon.rc");
    if std::fs::write(&rc_path, rc_text).is_err() {
        return;
    }
    let res_path = out_dir.join("icon.res");
    let ok = Command::new(&rc)
        .current_dir(&out_dir)
        .arg("/nologo")
        .arg(format!("/fo{}", res_path.display()))
        .arg(&rc_path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        println!("cargo:rustc-link-arg={}", res_path.display());
    } else {
        println!("cargo:warning=rc.exe 编译资源失败，exe 将没有图标");
    }
}

/// 找 Windows SDK 里最新一版的 rc.exe（PATH 里有就优先用）
fn find_rc() -> Option<PathBuf> {
    let kits = [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ];
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for root in kits {
        let Ok(rd) = std::fs::read_dir(root) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path().join("x64").join("rc.exe");
            if !p.is_file() {
                continue;
            }
            let key: Vec<u64> = e
                .file_name()
                .to_string_lossy()
                .split('.')
                .map(|s| s.parse::<u64>().unwrap_or(0))
                .collect();
            if best.as_ref().map(|(k, _)| key > *k).unwrap_or(true) {
                best = Some((key, p));
            }
        }
    }
    best.map(|(_, p)| p)
}
