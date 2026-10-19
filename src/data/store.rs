//! 落盘：各任务的累计统计快照（tokenbar-stats.json）。
//!
//! 日志回看窗口有限，任务切回去时数字不能凭空消失，所以每个任务看到过就记一份。

use super::model::{CachedConv, Stats};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const VERSION: u32 = 1;

pub fn load(path: &Path) -> HashMap<String, CachedConv> {
    let mut out = HashMap::new();
    let Ok(text) = fs::read_to_string(path) else {
        return out;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return out;
    };
    let Some(convs) = v.get("convs").and_then(|x| x.as_object()) else {
        return out;
    };
    for (k, c) in convs {
        out.insert(
            k.clone(),
            CachedConv {
                turns: u64_of(c, "turns") as usize,
                calls: u64_of(c, "calls") as usize,
                model: c
                    .get("model")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                updated: c.get("updated").and_then(|x| x.as_i64()).unwrap_or(0),
                total: c.get("total").map(stats_from_json).unwrap_or_default(),
                turn: c.get("turn").filter(|x| x.is_object()).map(stats_from_json),
            },
        );
    }
    out
}

/// 原子写：先写 .tmp 再改名，改名失败（文件被占用等）就退化为直接覆盖
pub fn save(path: &Path, cache: &HashMap<String, CachedConv>) -> bool {
    let mut convs = Map::new();
    for (k, c) in cache {
        convs.insert(
            k.clone(),
            json!({
                "turns": c.turns,
                "calls": c.calls,
                "model": c.model,
                "updated": c.updated,
                "total": stats_to_json(&c.total),
                "turn": c.turn.as_ref().map(stats_to_json),
            }),
        );
    }
    let body = json!({ "version": VERSION, "convs": convs }).to_string();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, body.as_bytes()).is_err() {
        return false;
    }
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::write(path, body.as_bytes());
        let _ = fs::remove_file(&tmp);
    }
    true
}

/// `--dump` 用的紧凑输出，保留原来的取整精度（外部脚本按这个口径对比）
pub fn stats_json_compact(s: Option<&Stats>) -> String {
    match s {
        Some(s) => format!(
            "{{\"calls\":{},\"input\":{},\"output\":{},\"cache\":{},\"rate\":{},\"ttft\":{:.0},\"duration\":{:.0},\"tps\":{:.1}}}",
            s.calls,
            s.input,
            s.output,
            s.cache_read,
            s.cache_rate
                .map(|r| format!("{:.1}", r))
                .unwrap_or_else(|| "null".into()),
            s.ttft_ms,
            s.duration_ms,
            s.tps
        ),
        None => "null".to_string(),
    }
}

fn stats_to_json(s: &Stats) -> Value {
    json!({
        "calls": s.calls,
        "input": s.input,
        "output": s.output,
        "cache": s.cache_read,
        "rate": s.cache_rate,
        "ttft": s.ttft_ms,
        "duration": s.duration_ms,
        "tps": s.tps,
    })
}

fn stats_from_json(v: &Value) -> Stats {
    Stats {
        calls: u64_of(v, "calls") as usize,
        input: u64_of(v, "input"),
        output: u64_of(v, "output"),
        cache_read: u64_of(v, "cache"),
        cache_rate: v.get("rate").and_then(|x| x.as_f64()).map(|x| x as f32),
        ttft_ms: v.get("ttft").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        duration_ms: v.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        tps: v.get("tps").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
    }
}

fn u64_of(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("tokenbar_store_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("tokenbar-stats.json");

        let mut cache = HashMap::new();
        cache.insert(
            "conv-1".to_string(),
            CachedConv {
                turns: 2,
                calls: 3,
                model: "m".into(),
                updated: 7,
                total: Stats {
                    calls: 3,
                    input: 35000,
                    output: 1000,
                    cache_read: 19000,
                    cache_rate: Some(54.3),
                    ttft_ms: 733.0,
                    duration_ms: 7400.0,
                    tps: 238.1,
                },
                turn: Some(Stats {
                    calls: 1,
                    input: 13000,
                    output: 200,
                    ..Default::default()
                }),
            },
        );
        assert!(save(&path, &cache));

        let back = load(&path);
        let c = back.get("conv-1").expect("应读回");
        assert_eq!((c.turns, c.calls, c.updated), (2, 3, 7));
        assert_eq!(c.total.input, 35000);
        assert!((c.total.cache_rate.unwrap() - 54.3).abs() < 0.01);
        assert_eq!(c.turn.as_ref().unwrap().output, 200);

        // 坏文件不该 panic，只当作空
        fs::write(&path, "{ not json").unwrap();
        assert!(load(&path).is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
