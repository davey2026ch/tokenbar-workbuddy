//! 采集器：以 250ms 的节奏轮询日志与 transcript，聚合出"本轮 / 累计"。
//!
//! 取数优先级：
//! 1. 日志现算（有时间类指标）
//! 2. 落盘快照（日志过期时用，累计行标"缓存"）
//! 3. transcript 全量求和（只剩 token / 缓存，时间显示"—"）
//! 4. 都没有 → 空态

use super::model::{CachedConv, Call, Snapshot, Stats, Status, Turn};
use super::parse::{self, LogEvent};
use super::store;
use super::tail::Tails;
use chrono::{Duration as ChronoDuration, Local};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// 内存里最多保留多少轮；超了就砍掉最早的一半
const MAX_TURNS: usize = 4000;
/// 开始事件超过这个时间还没等到结束，就认为它已经断了，不再显示"生成中"
const OPEN_TIMEOUT_MS: i64 = 10 * 60 * 1000;
/// 日志回看几天：切换到一个更早开始的会话时也能看到它的累计数据
const LOOKBACK_DAYS: i64 = 5;
/// transcript 所在目录的查找结果缓存多久
const DIRS_TTL_MS: i64 = 4000;
/// 快照写盘的最小间隔，避免每帧都 IO
const CACHE_SAVE_INTERVAL_MS: i64 = 5_000;
/// 状态机最后一次转换超过这么久还没动静，就不再当"在干活"（防止日志中断后一直亮着）
const ACTIVE_FRESH_MS: i64 = 30 * 60 * 1000;

pub struct Tracker {
    logs: Tails,
    transcripts: Tails,
    /// 会话状态机日志（sdk/conversations/<会话>.log）
    states: Tails,
    /// (会话, 状态日志路径, 上次查找时间)
    state_log: Option<(String, PathBuf, i64)>,
    /// (会话, 最后状态时间, 是否在干活)
    conv_state: Option<(String, i64, bool)>,
    seen_usage: HashSet<String>,
    cache_by_req: HashMap<String, (u64, u64)>,
    turns: Vec<Turn>,
    turn_index: HashMap<String, usize>,
    call_index: HashMap<(String, String), (usize, usize)>,
    conv_dirs: HashMap<String, (Vec<PathBuf>, i64)>,
    /// 老任务的历史统计（内存缓存，避免每帧重读 transcript）
    history: HashMap<String, (i64, Option<(Stats, String)>)>,
    /// 各任务的累计快照（持久化到 tokenbar-stats.json）
    cache: HashMap<String, CachedConv>,
    cache_path: Option<PathBuf>,
    cache_dirty: bool,
    cache_saved_at: i64,
    route_path: Option<PathBuf>,
    current_conv: Option<String>,
    /// 停在新任务页 / 项目 / 日历这类"已经离开会话"的页面：
    /// WorkBuddy 自己会把当前会话清掉，这里跟着清，别再把上一个任务摆在卡片上
    off_conv: bool,
    home: PathBuf,
}

impl Tracker {
    pub fn new() -> Self {
        Self::with_root_and_cache(super::home_dir(), None)
    }

    /// 带持久化统计快照的实例（快照文件一般放在 ini 同目录）
    pub fn new_with_cache(cache_path: PathBuf) -> Self {
        Self::with_root_and_cache(super::home_dir(), Some(cache_path))
    }

    /// 指定 home（其中包含 .workbuddy），测试用
    #[cfg(test)]
    pub fn with_root(home: PathBuf) -> Self {
        Self::with_root_and_cache(home, None)
    }

    /// 指定 home（其中包含 .workbuddy）与快照路径
    pub fn with_root_and_cache(home: PathBuf, cache_path: Option<PathBuf>) -> Self {
        let cache = cache_path
            .as_ref()
            .map(|p| store::load(p))
            .unwrap_or_default();
        Self {
            logs: Tails::default(),
            transcripts: Tails::default(),
            states: Tails::default(),
            state_log: None,
            conv_state: None,
            seen_usage: HashSet::new(),
            cache_by_req: HashMap::new(),
            turns: Vec::new(),
            turn_index: HashMap::new(),
            call_index: HashMap::new(),
            conv_dirs: HashMap::new(),
            history: HashMap::new(),
            cache,
            cache_path,
            cache_dirty: false,
            cache_saved_at: 0,
            route_path: None,
            current_conv: None,
            off_conv: false,
            home,
        }
    }

    fn wb_root(&self) -> PathBuf {
        self.home.join(".workbuddy")
    }

    pub fn poll(&mut self) {
        self.refresh_current_conv();
        self.scan_logs();
        let active = self.active_conv();
        if let Some(conv) = active.clone() {
            self.scan_transcripts(&conv);
            self.scan_conv_state(&conv);
        }
        self.attach_cache();
        if let Some(conv) = active {
            if let Some(a) = self.aggregate(&conv) {
                self.note_cache(&conv, &a);
            }
        }
        self.flush_cache();
    }

    // ---------- 日志 ----------

    fn scan_logs(&mut self) {
        let now = Local::now();
        let dirs: Vec<PathBuf> = (0..LOOKBACK_DAYS)
            .map(|i| {
                self.wb_root().join("logs").join(
                    (now - ChronoDuration::days(i))
                        .format("%Y-%m-%d")
                        .to_string(),
                )
            })
            .filter(|d| d.is_dir())
            .collect();
        for dir in dirs {
            let Ok(rd) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("log") {
                    continue;
                }
                for line in self.logs.read_new(&path) {
                    // 先过滤再解析：日志里绝大部分行与 qu_ 无关
                    if !line.contains("qua.start: {") && !line.contains("qua.end: {") {
                        continue;
                    }
                    if let Some(ev) = parse::log_event(&line) {
                        self.handle_log(&ev);
                    }
                }
            }
        }
    }

    fn handle_log(&mut self, ev: &LogEvent) {
        let t_idx = self.ensure_turn(&ev.conv, &ev.req, &ev.model, ev.ts_ms);
        {
            let turn = &mut self.turns[t_idx];
            turn.last_activity = turn.last_activity.max(ev.ts_ms);
            if ev.model != "unknown" && !ev.model.is_empty() {
                turn.model = ev.model.clone();
            }
        }

        let key = (ev.req.clone(), ev.msg.clone());
        let existing = self.call_index.get(&key).copied();
        if ev.end {
            let call = Call {
                start_ms: ev.ts_ms - ev.total_ms as i64,
                end_ms: ev.ts_ms,
                ttft_ms: ev.ttft_ms,
                total_ms: ev.total_ms,
                prompt: ev.prompt,
                completion: ev.completion,
                done: true,
            };
            match existing {
                Some((ti, ci)) => {
                    let t = &mut self.turns[ti];
                    t.calls[ci] = call;
                    t.last_activity = t.last_activity.max(ev.ts_ms);
                }
                None => {
                    let ti = t_idx;
                    let ci = self.turns[ti].calls.len();
                    self.turns[ti].calls.push(call);
                    self.call_index.insert(key, (ti, ci));
                }
            }
        } else if existing.is_none() {
            let call = Call {
                start_ms: ev.ts_ms,
                end_ms: 0,
                ttft_ms: 0.0,
                total_ms: 0.0,
                prompt: 0,
                completion: 0,
                done: false,
            };
            let ti = t_idx;
            let ci = self.turns[ti].calls.len();
            self.turns[ti].calls.push(call);
            self.call_index.insert(key, (ti, ci));
        }
    }

    fn ensure_turn(&mut self, conv: &str, req: &str, model: &str, ts_ms: i64) -> usize {
        if let Some(i) = self.turn_index.get(req) {
            return *i;
        }
        self.turns.push(Turn {
            conv: conv.to_string(),
            id: req.to_string(),
            model: model.to_string(),
            first_start: ts_ms,
            last_activity: ts_ms,
            calls: Vec::new(),
            cache_read: 0,
            cache_create: 0,
            cache_known: false,
        });
        let idx = self.turns.len() - 1;
        self.turn_index.insert(req.to_string(), idx);
        if self.turns.len() > MAX_TURNS {
            self.compact();
        }
        idx
    }

    fn compact(&mut self) {
        let drop_n = self.turns.len() - MAX_TURNS / 2;
        self.turns.drain(0..drop_n);
        self.turn_index.clear();
        self.call_index.clear();
        for (i, t) in self.turns.iter().enumerate() {
            self.turn_index.insert(t.id.clone(), i);
        }
    }

    fn active_conv(&self) -> Option<String> {
        // 已经离开会话（新任务页等）：不要把上一个任务当成"当前任务"
        if self.off_conv {
            return None;
        }
        if let Some(c) = &self.current_conv {
            return Some(c.clone());
        }
        self.turns
            .iter()
            .max_by_key(|t| t.last_activity)
            .map(|t| t.conv.clone())
    }

    /// WorkBuddy 把"当前停留的会话"写在 <user>/startup-route.json 里，
    /// 形如 {"path":"/conversation/<uuid>", ...}，切换任务时会更新。
    fn refresh_current_conv(&mut self) {
        if self.route_path.is_none() {
            self.route_path = find_route_file(&self.wb_root());
        }
        let Some(path) = self.route_path.clone() else {
            return;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            return;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return;
        };
        let Some(route) = v.get("path").and_then(|x| x.as_str()) else {
            return;
        };
        match parse_route(route) {
            Route::Conv(id) => {
                self.off_conv = false;
                if self.current_conv.as_deref() != Some(id.as_str()) {
                    self.current_conv = Some(id);
                }
            }
            Route::LeftConv => {
                self.off_conv = true;
                self.current_conv = None;
            }
            // 资料库 / 专家 / 插件这类页面 WorkBuddy 不动当前会话，我们也不动
            Route::Other => {}
        }
    }

    // ---------- transcript（缓存命中） ----------

    fn scan_transcripts(&mut self, conv: &str) {
        let now_ms = Local::now().timestamp_millis();
        let fresh = self
            .conv_dirs
            .get(conv)
            .map(|(_, t)| now_ms - *t < DIRS_TTL_MS)
            .unwrap_or(false);
        if !fresh {
            let files = self
                .find_transcript(conv)
                .map(|(main, subs)| {
                    let mut v = vec![main];
                    v.extend(subs);
                    v
                })
                .unwrap_or_default();
            self.conv_dirs.insert(conv.to_string(), (files, now_ms));
        }

        let files = self
            .conv_dirs
            .get(conv)
            .map(|(f, _)| f.clone())
            .unwrap_or_default();
        for path in files {
            for line in self.transcripts.read_new(&path) {
                self.apply_usage(&line);
            }
        }
    }

    fn find_transcript(&self, conv: &str) -> Option<(PathBuf, Vec<PathBuf>)> {
        let rd = fs::read_dir(self.wb_root().join("projects")).ok()?;
        for entry in rd.flatten() {
            let dir = entry.path();
            let main = dir.join(format!("{}.jsonl", conv));
            if !main.is_file() {
                continue;
            }
            let mut subs = Vec::new();
            let subdir = dir.join(conv).join("subagents");
            if let Ok(srd) = fs::read_dir(&subdir) {
                for s in srd.flatten() {
                    let p = s.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        subs.push(p);
                    }
                }
            }
            return Some((main, subs));
        }
        None
    }

    /// 读会话状态机日志，知道这一轮是"在干活"还是"已结束"。
    /// 模型调用之间跑工具时没有 qua 事件，光看调用会误判成空闲。
    fn scan_conv_state(&mut self, conv: &str) {
        let now_ms = Local::now().timestamp_millis();
        let cached = self
            .state_log
            .as_ref()
            .filter(|(c, _, t)| c == conv && now_ms - *t < DIRS_TTL_MS)
            .map(|(_, p, _)| p.clone());
        let path = match cached {
            Some(p) => p,
            None => match self.find_state_log(conv) {
                Some(p) => {
                    self.state_log = Some((conv.to_string(), p.clone(), now_ms));
                    p
                }
                None => return,
            },
        };
        for line in self.states.read_new(&path) {
            if let Some((ts, active)) = parse::conv_state(&line) {
                self.conv_state = Some((conv.to_string(), ts, active));
            }
        }
    }

    fn find_state_log(&self, conv: &str) -> Option<PathBuf> {
        let now = Local::now();
        for i in 0..2 {
            let day = (now - ChronoDuration::days(i)).format("%Y-%m-%d").to_string();
            let p = self
                .wb_root()
                .join("logs")
                .join(day)
                .join("sdk")
                .join("conversations")
                .join(format!("{}.log", conv));
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }

    /// 这个会话当前是不是还在跑（思考 / 工具 / 生成都算）
    fn active_state(&self, conv: &str) -> bool {
        match &self.conv_state {
            Some((c, ts, active)) if c == conv => {
                *active && Local::now().timestamp_millis() - *ts < ACTIVE_FRESH_MS
            }
            _ => false,
        }
    }

    /// 日志翻不到了（超出回看窗口 / 被轮转）时，用 transcript 里每条回复的 usage 求和，
    /// 至少把输入 / 输出 / 缓存取出来；时间类指标无从得知，留 0（界面显示 —）。
    fn transcript_totals(&mut self, conv: &str) -> Option<(Stats, String)> {
        let (main, _) = self.find_transcript(conv)?;
        let mtime = fs::metadata(&main)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if let Some((m, cached)) = self.history.get(conv) {
            if *m == mtime {
                return cached.clone();
            }
        }
        let text = fs::read_to_string(&main).ok()?;
        let mut seen: HashSet<String> = HashSet::new();
        let mut stats = Stats::default();
        let mut model = String::new();
        for line in text.lines() {
            let Some(u) = parse::usage(line) else {
                continue;
            };
            if !seen.insert(u.msg_id.clone()) {
                continue;
            }
            // 时间类指标没有，用量为 0 的记录也不计入调用次数
            if u.input + u.output == 0 {
                continue;
            }
            stats.calls += 1;
            stats.input += u.input;
            stats.output += u.output;
            stats.cache_read += u.cache_read;
            if !u.model.is_empty() {
                model = u.model;
            }
        }
        if stats.calls == 0 {
            self.history.insert(conv.to_string(), (mtime, None));
            return None;
        }
        stats.cache_rate = hit_rate(stats.cache_read, stats.input);
        let out = (stats, model);
        self.history
            .insert(conv.to_string(), (mtime, Some(out.clone())));
        Some(out)
    }

    /// 把增量读到的一条 usage 记到对应请求上（同一行重复出现要跳过）
    fn apply_usage(&mut self, line: &str) {
        let Some(u) = parse::usage(line) else {
            return;
        };
        if u.is_empty() {
            return;
        }
        let dedupe = format!(
            "{}|{}|{}|{}|{}",
            u.msg_id, u.input, u.output, u.cache_read, u.cache_create
        );
        if !self.seen_usage.insert(dedupe) {
            return;
        }
        let Some(req) = u.request_id else {
            return;
        };
        let e = self.cache_by_req.entry(req).or_insert((0, 0));
        e.0 += u.cache_read;
        e.1 += u.cache_create;
    }

    fn attach_cache(&mut self) {
        for t in self.turns.iter_mut() {
            if let Some((read, create)) = self.cache_by_req.get(&t.id) {
                t.cache_read = *read;
                t.cache_create = *create;
                t.cache_known = true;
            }
        }
    }

    // ---------- 聚合 ----------

    #[cfg(test)]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for t in &self.turns {
            out.push_str(&format!(
                "turn req={} conv={} first={} last={} calls={} cache={} known={}\n",
                t.id,
                t.conv,
                t.first_start,
                t.last_activity,
                t.calls.len(),
                t.cache_read,
                t.cache_known
            ));
            for c in &t.calls {
                out.push_str(&format!(
                    "   call start={} end={} done={} prompt={} comp={}\n",
                    c.start_ms, c.end_ms, c.done, c.prompt, c.completion
                ));
            }
        }
        out
    }

    pub fn snapshot(&mut self) -> Snapshot {
        let Some(current) = self.active_conv() else {
            return Snapshot::default();
        };
        if let Some(a) = self.aggregate(&current) {
            return Snapshot {
                status: if a.generating {
                    Status::Generating
                } else {
                    Status::Idle
                },
                active: a.active,
                model: a.model,
                turn: Some(a.turn),
                total: Some(a.total),
                turns_count: a.turns,
                elapsed_ms: a.elapsed_ms,
                spark: a.spark,
                current_conv: current,
                stale: false,
            };
        }
        // 日志里没有这个任务了：先用落盘的快照，再退到 transcript 里的历史用量
        if let Some(c) = self.cache.get(&current) {
            return Snapshot {
                status: Status::Cached,
                active: false,
                model: c.model.clone(),
                turn: c.turn.clone(),
                total: Some(c.total.clone()),
                turns_count: c.turns,
                elapsed_ms: 0.0,
                spark: Vec::new(),
                current_conv: current,
                stale: true,
            };
        }
        if let Some((stats, model)) = self.transcript_totals(&current) {
            return Snapshot {
                status: Status::Cached,
                active: false,
                model,
                turn: None,
                total: Some(stats),
                turns_count: 0,
                elapsed_ms: 0.0,
                spark: Vec::new(),
                current_conv: current,
                stale: true,
            };
        }
        // 当前任务确实还没有调用记录（新会话）
        Snapshot {
            status: Status::NoData,
            current_conv: current,
            ..Default::default()
        }
    }

    /// 按日志里现成的调用记录算出某个任务的本轮/累计
    fn aggregate(&self, conv: &str) -> Option<Agg> {
        let conv_turns: Vec<&Turn> = self.turns.iter().filter(|t| t.conv == conv).collect();
        if conv_turns.is_empty() {
            return None;
        }
        let now = Local::now().timestamp_millis();
        let latest = conv_turns.iter().max_by_key(|t| t.last_activity).copied()?;
        let generating = latest
            .calls
            .iter()
            .any(|c| !c.done && now - c.start_ms < OPEN_TIMEOUT_MS);
        // 没有进行中的调用，但会话状态机还在 working（跑工具/思考）→ 也算"在跑"，
        // 耗时继续走，圆点不要变灰
        let active = self.active_state(conv);
        let running = generating || active;
        let turn = stats_of(latest, now, running);

        // 累计：只统计已完成的调用
        let mut total = Stats::default();
        let mut gen_ms = 0.0f32;
        let mut ttft_sum = 0.0f32;
        let mut ttft_n = 0;
        for t in &conv_turns {
            for c in &t.calls {
                if !c.done {
                    continue;
                }
                total.count_call(c);
                gen_ms += (c.total_ms - c.ttft_ms).max(0.0);
                if c.ttft_ms > 0.0 {
                    ttft_sum += c.ttft_ms;
                    ttft_n += 1;
                }
            }
            total.cache_read += t.cache_read;
        }
        total.tps_from(gen_ms);
        if ttft_n > 0 {
            total.ttft_ms = ttft_sum / ttft_n as f32;
        }
        // 累计耗时用各轮 wall-clock 之和更贴近"这个任务花了多久"
        total.duration_ms = conv_turns
            .iter()
            .map(|t| {
                let end = if running && t.id == latest.id {
                    now
                } else {
                    t.last_activity
                };
                (end - t.first_start).max(0) as f32
            })
            .sum();
        total.cache_rate = hit_rate(total.cache_read, total.input);

        let spark: Vec<f32> = conv_turns
            .iter()
            .filter(|t| !t.calls.is_empty())
            .rev()
            .take(14)
            .map(|t| t.calls.iter().map(|c| c.completion as f32).sum::<f32>())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        Some(Agg {
            turn,
            total,
            turns: conv_turns.len(),
            model: latest.model.clone(),
            elapsed_ms: (now - latest.first_start).max(0) as f32,
            spark,
            generating,
            active: running,
        })
    }

    fn note_cache(&mut self, conv: &str, a: &Agg) {
        let now = Local::now().timestamp_millis();
        let entry = self.cache.entry(conv.to_string()).or_default();
        let changed = entry.turns != a.turns
            || entry.total.input != a.total.input
            || entry.total.output != a.total.output
            || entry.total.duration_ms as i64 != a.total.duration_ms as i64
            || entry.model != a.model;
        entry.turns = a.turns;
        entry.calls = a.total.calls;
        entry.total = a.total.clone();
        entry.turn = Some(a.turn.clone());
        entry.model = a.model.clone();
        entry.updated = now;
        if changed {
            self.cache_dirty = true;
        }
    }

    /// 最多每 5 秒写一次，避免频繁 IO；进程退出前由 App 调 flush。
    fn flush_cache(&mut self) {
        if !self.cache_dirty {
            return;
        }
        let now = Local::now().timestamp_millis();
        if now - self.cache_saved_at < CACHE_SAVE_INTERVAL_MS {
            return;
        }
        if self.save_cache() {
            self.cache_dirty = false;
            self.cache_saved_at = now;
        }
    }

    pub fn save_cache(&self) -> bool {
        match &self.cache_path {
            Some(path) => store::save(path, &self.cache),
            None => false,
        }
    }
}

struct Agg {
    turn: Stats,
    total: Stats,
    turns: usize,
    model: String,
    elapsed_ms: f32,
    spark: Vec<f32>,
    generating: bool,
    active: bool,
}

fn stats_of(t: &Turn, now: i64, generating: bool) -> Stats {
    let mut s = Stats::default();
    let mut gen_ms = 0.0f32;
    let mut first_ttft: Option<f32> = None;
    let mut last_end = t.first_start;
    for c in &t.calls {
        if !c.done {
            continue;
        }
        s.count_call(c);
        gen_ms += (c.total_ms - c.ttft_ms).max(0.0);
        if first_ttft.is_none() && c.ttft_ms > 0.0 {
            first_ttft = Some(c.ttft_ms);
        }
        last_end = last_end.max(c.end_ms);
    }
    s.ttft_ms = first_ttft.unwrap_or(0.0);
    s.duration_ms = if generating {
        (now - t.first_start) as f32
    } else {
        (last_end - t.first_start).max(0) as f32
    };
    s.tps_from(gen_ms);
    s.cache_read = t.cache_read;
    s.cache_rate = if t.cache_known {
        hit_rate(t.cache_read, s.input)
    } else {
        None
    };
    s
}

fn hit_rate(read: u64, input: u64) -> Option<f32> {
    if read == 0 || input == 0 {
        return None;
    }
    let denom = if read <= input { input } else { input + read };
    Some((read as f32 / denom as f32 * 100.0).min(99.9))
}

/// startup-route.json 里的 path 该怎么理解
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// 正停在某个会话上：/conversation/<id>，项目内会话是 /task/<id>
    Conv(String),
    /// 已经离开会话：新任务页（/ 或 /home）、项目、日历
    LeftConv,
    /// 其它页面（资料库、专家、插件…）：保持原样
    Other,
}

fn parse_route(route: &str) -> Route {
    // 新版 WorkBuddy 会在 id 后面带查询参数（如 ?startIntentToken=3），要剥掉
    let path = route.split(['?', '#']).next().unwrap_or(route).trim();
    for prefix in ["/conversation/", "/task/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            let id = rest.split('/').next().unwrap_or(rest).trim();
            // WorkBuddy 的路由表里 /task/:taskId 与 /conversation/:id 一样指向会话
            return if id.len() >= 8 {
                Route::Conv(id.to_string())
            } else {
                Route::Other
            };
        }
    }
    // 路由表把 index(/) 与 home 都标成 clearCurrentConversation，
    // projects / calendar 同样会把"当前会话"清掉
    let left = path == "/"
        || path == "/home"
        || path == "/projects"
        || path.starts_with("/projects/")
        || path == "/calendar"
        || path.starts_with("/calendar/");
    if left {
        Route::LeftConv
    } else {
        Route::Other
    }
}

/// <home>/.workbuddy/<用户 id>/startup-route.json
fn find_route_file(wb_root: &Path) -> Option<PathBuf> {
    for e in fs::read_dir(wb_root).ok()?.flatten() {
        let p = e.path().join("startup-route.json");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_strips_query_string() {
        // 新版 WorkBuddy 的 startup-route.json 会写成
        // "/conversation/<uuid>?startIntentToken=3"，查询参数必须剥掉
        let dir = std::env::temp_dir().join(format!("tokenbar-test-{}", std::process::id()));
        let uid = dir.join(".workbuddy").join("u1");
        fs::create_dir_all(&uid).unwrap();
        fs::write(
            uid.join("startup-route.json"),
            r#"{"path":"/conversation/591fb422-bc44-42d1-abd9-3a0c252a01e1?startIntentToken=3"}"#,
        )
        .unwrap();
        let mut t = Tracker::with_root(dir.clone());
        t.refresh_current_conv();
        assert_eq!(
            t.current_conv.as_deref(),
            Some("591fb422-bc44-42d1-abd9-3a0c252a01e1")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hit_rate_never_exceeds_full() {
        assert_eq!(hit_rate(0, 100), None);
        assert_eq!(hit_rate(100, 0), None);
        assert!((hit_rate(5000, 10000).unwrap() - 50.0).abs() < 0.01);
        // 缓存比输入还多时，分母用两者之和
        assert!(hit_rate(19000, 13000).unwrap() < 100.0);
    }

    #[test]
    fn parses_route_paths() {
        let uuid = "9a89b660-bdc2-4bd0-ae14-e4a51268bdf4";
        // 会话页：两种写法都算
        assert_eq!(
            parse_route(&format!("/conversation/{}", uuid)),
            Route::Conv(uuid.to_string())
        );
        assert_eq!(
            parse_route(&format!("/conversation/{}?startIntentToken=3", uuid)),
            Route::Conv(uuid.to_string())
        );
        assert_eq!(
            parse_route(&format!("/task/{}", uuid)),
            Route::Conv(uuid.to_string())
        );
        // 新任务页 = 已经离开会话
        assert_eq!(parse_route("/"), Route::LeftConv);
        assert_eq!(parse_route("/home"), Route::LeftConv);
        assert_eq!(parse_route("/projects?projectId=p1"), Route::LeftConv);
        assert_eq!(parse_route("/calendar"), Route::LeftConv);
        // 其它页面保持原样
        assert_eq!(parse_route("/library/my-files"), Route::Other);
        assert_eq!(parse_route("/experts"), Route::Other);
        assert_eq!(parse_route("/conversation/"), Route::Other);
    }

    /// 用户点"新对话"后，卡片要立刻空掉，不能还挂着上一个任务的数字
    #[test]
    fn new_task_page_clears_previous_conv() {
        let dir = std::env::temp_dir().join(format!("tokenbar-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let uid = dir.join(".workbuddy").join("u1");
        fs::create_dir_all(&uid).unwrap();

        // 先停在某个会话上
        fs::write(
            uid.join("startup-route.json"),
            r#"{"path":"/conversation/9a89b660-bdc2-4bd0-ae14-e4a51268bdf4","savedAt":1}"#,
        )
        .unwrap();
        let mut t = Tracker::with_root(dir.clone());
        t.refresh_current_conv();
        assert_eq!(
            t.current_conv.as_deref(),
            Some("9a89b660-bdc2-4bd0-ae14-e4a51268bdf4")
        );

        // 回到新任务页：当前会话要清掉，且不能回退到"最近活跃的那个任务"
        fs::write(
            uid.join("startup-route.json"),
            r#"{"path":"/home","savedAt":2}"#,
        )
        .unwrap();
        t.refresh_current_conv();
        assert!(t.current_conv.is_none(), "新任务页应当没有当前会话");
        assert!(t.active_conv().is_none(), "不能退回到上一个任务");

        // 再回到那个会话：恢复
        fs::write(
            uid.join("startup-route.json"),
            r#"{"path":"/conversation/9a89b660-bdc2-4bd0-ae14-e4a51268bdf4","savedAt":3}"#,
        )
        .unwrap();
        t.refresh_current_conv();
        assert_eq!(
            t.current_conv.as_deref(),
            Some("9a89b660-bdc2-4bd0-ae14-e4a51268bdf4")
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
