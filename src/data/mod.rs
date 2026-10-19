//! 数据采集：尾部读取 WorkBuddy 的 SSE 监控日志（首字/耗时/Token）
//! 以及会话 transcript（缓存命中 token），聚合成"本轮 / 累计"两套指标。
//!
//! - `model`   数据模型
//! - `parse`   日志行 / transcript 行的解析
//! - `tail`    文件增量读取
//! - `store`   统计快照落盘
//! - `tracker` 采集与聚合（对外只有 `Tracker`）

mod model;
mod parse;
mod store;
mod tail;
mod tracker;

pub use model::{Snapshot, Stats, Status};
pub use store::stats_json_compact;
pub use tracker::Tracker;

use std::path::PathBuf;

/// 用户主目录（Windows 用 USERPROFILE，其余平台用 HOME）
pub(crate) fn home_dir() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests;
