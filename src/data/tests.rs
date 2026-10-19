use crate::data::{Status, Tracker};
use chrono::Local;
use std::fs;
use std::path::PathBuf;

fn tmp_home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tokenbar_test_{}_{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".workbuddy/logs")).unwrap();
    fs::create_dir_all(dir.join(".workbuddy/projects/fake")).unwrap();
    dir
}

fn log_line(ts: &str, kind: &str, body: &str) -> String {
    format!(
        "[{}] [Info] [pid=1] [ModelProvider]  [SseMonitor#01] {}: {}\n",
        ts, kind, body
    )
}

fn start_json(req: &str, msg: &str, conv: &str, typ: &str) -> String {
    format!(
        "{{\"conversation_id\":\"{}\",\"conversation_request_id\":\"{}\",\"prompt_request_id\":\"\",\"trace_id\":\"\",\"message_id\":\"{}\",\"model\":\"test-model\",\"agent_name\":\"cli\",\"scene\":\"craft\",\"request_type\":\"{}\",\"report_version\":\"1\",\"report_source\":\"agent-cli\"}}",
        conv, req, msg, typ
    )
}

#[allow(clippy::too_many_arguments)]
fn end_json(
    req: &str,
    msg: &str,
    conv: &str,
    typ: &str,
    total: i64,
    ttft: i64,
    prompt: i64,
    comp: i64,
) -> String {
    format!(
        "{{\"conversation_id\":\"{}\",\"conversation_request_id\":\"{}\",\"prompt_request_id\":\"\",\"trace_id\":\"\",\"message_id\":\"{}\",\"model\":\"test-model\",\"agent_name\":\"cli\",\"scene\":\"craft\",\"request_type\":\"{}\",\"report_version\":\"1\",\"report_source\":\"agent-cli\",\"http_begin_status_code\":\"200\",\"http_end_status_code\":\"200\",\"model_error\":\"\",\"model_error_code\":\"\",\"finish_flag\":\"done\",\"finish_reason\":\"stop\",\"total_cost_ms\":\"{}\",\"sse_connect_cost_ms\":\"100\",\"ttft_ms\":\"{}\",\"text_first_cost_ms\":\"0\",\"text_total_cost_ms\":\"0\",\"think_first_cost_ms\":\"0\",\"think_total_cost_ms\":\"0\",\"total_bytes\":\"1000\",\"total_chunks\":\"10\",\"completion_tokens\":\"{}\",\"prompt_tokens\":\"{}\",\"total_tokens\":\"{}\",\"text_content_bytes\":\"0\",\"text_content_chars\":\"0\",\"text_total_tokens\":\"0\",\"think_content_bytes\":\"0\",\"think_content_chars\":\"0\",\"think_total_tokens\":\"0\",\"recv_length\":\"1000\",\"origin_length\":\"1000\"}}",
        conv, req, msg, typ, total, ttft, comp, prompt, prompt + comp
    )
}

fn assistant_json(req: &str, msg_id: &str, input: u64, output: u64, read: u64) -> String {
    format!(
        "{{\"type\":\"assistant\",\"uuid\":\"u-{}\",\"session_id\":\"c1\",\"message\":{{\"id\":\"{}\",\"role\":\"assistant\",\"model\":\"test-model\",\"content\":[],\"usage\":{{\"input_tokens\":{},\"output_tokens\":{},\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":{}}}}},\"_requestId\":\"{}\"}}\n",
        msg_id, msg_id, input, output, read, req
    )
}

#[test]
fn aggregates_turns_and_cache() {
    let home = tmp_home("agg");
    let conv = "conv-1";
    let date = Local::now().format("%Y-%m-%d").to_string();
    let log_dir = home.join(".workbuddy/logs").join(&date);
    fs::create_dir_all(&log_dir).unwrap();

    let base = Local::now() - chrono::Duration::minutes(20);
    let t = |ms: i64| {
        (base + chrono::Duration::milliseconds(ms))
            .format("%Y/%m/%d %H:%M:%S%.3f")
            .to_string()
    };

    // 第一轮：两次调用；另有一条标题请求（应被忽略）
    let mut log = String::new();
    log += &log_line(&t(0), "qua.start", &start_json("R1", "M1", conv, "chat"));
    log += &log_line(
        &t(3000),
        "qua.end",
        &end_json("R1", "M1", conv, "chat", 3000, 1000, 10000, 500),
    );
    log += &log_line(&t(3500), "qua.start", &start_json("R1", "M2", conv, "chat"));
    log += &log_line(
        &t(5500),
        "qua.end",
        &end_json("R1", "M2", conv, "chat", 2000, 500, 12000, 300),
    );
    log += &log_line(
        &t(5600),
        "qua.end",
        &end_json(
            "Rtopic",
            "M9",
            "conv-topic",
            "conversation_topic",
            900,
            100,
            500,
            20,
        ),
    );
    // 第二轮
    log += &log_line(&t(6000), "qua.start", &start_json("R2", "M3", conv, "chat"));
    log += &log_line(
        &t(7400),
        "qua.end",
        &end_json("R2", "M3", conv, "chat", 1400, 800, 13000, 200),
    );
    fs::write(log_dir.join("Tokenbar__test.log"), log).unwrap();

    // transcript：缓存命中
    let mut jsonl = String::new();
    jsonl += &assistant_json("R1", "M1", 10000, 500, 6000);
    jsonl += &assistant_json("R1", "M2", 12000, 300, 9000);
    jsonl += &assistant_json("R2", "M3", 13000, 200, 4000);
    fs::write(
        home.join(".workbuddy/projects/fake")
            .join(format!("{}.jsonl", conv)),
        jsonl,
    )
    .unwrap();

    let mut tr = Tracker::with_root(home.clone());
    tr.poll();
    let s = tr.snapshot();

    assert_eq!(s.status, Status::Idle);
    assert_eq!(s.turns_count, 2);
    let turn = s.turn.unwrap();
    assert_eq!(turn.input, 13000);
    assert_eq!(turn.output, 200);
    assert_eq!(turn.cache_read, 4000);
    assert!((turn.ttft_ms - 800.0).abs() < 0.1);
    assert!(
        (turn.duration_ms - 1400.0).abs() < 1.0,
        "duration={} elapsed={} tps={}
{}",
        turn.duration_ms,
        s.elapsed_ms,
        turn.tps,
        tr.dump()
    );
    assert!((turn.tps - 333.3).abs() < 2.0, "tps={}", turn.tps);
    let rate = turn.cache_rate.unwrap();
    assert!((rate - 30.77).abs() < 0.5, "rate={}", rate);

    let total = s.total.unwrap();
    assert_eq!(total.input, 35000);
    assert_eq!(total.output, 1000);
    assert_eq!(total.cache_read, 19000);
    assert_eq!(total.calls, 3);
    assert!(
        (total.tps - (1000.0 / 4.1)).abs() < 1.0,
        "total tps={}",
        total.tps
    );

    // 两轮累计墙钟耗时
    assert!(
        (total.duration_ms - (5500.0 + 1400.0)).abs() < 2.0,
        "total duration={}",
        total.duration_ms
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn unfinished_turn_is_generating() {
    let home = tmp_home("live");
    let conv = "conv-2";
    let date = Local::now().format("%Y-%m-%d").to_string();
    let log_dir = home.join(".workbuddy/logs").join(&date);
    fs::create_dir_all(&log_dir).unwrap();
    let now = Local::now() - chrono::Duration::seconds(5);
    let ts = now.format("%Y/%m/%d %H:%M:%S%.3f").to_string();
    let mut log = String::new();
    log += &log_line(&ts, "qua.start", &start_json("R9", "M1", conv, "chat"));
    fs::write(log_dir.join("a.log"), log).unwrap();

    let mut tr = Tracker::with_root(home.clone());
    tr.poll();
    let s = tr.snapshot();
    assert_eq!(s.status, Status::Generating);
    assert!(s.elapsed_ms >= 4000.0, "elapsed={}", s.elapsed_ms);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn follows_current_route() {
    let home = tmp_home("route");
    let date = Local::now().format("%Y-%m-%d").to_string();
    let log_dir = home.join(".workbuddy/logs").join(&date);
    fs::create_dir_all(&log_dir).unwrap();

    let base = Local::now() - chrono::Duration::minutes(30);
    let t = |ms: i64| {
        (base + chrono::Duration::milliseconds(ms))
            .format("%Y/%m/%d %H:%M:%S%.3f")
            .to_string()
    };
    let mut log = String::new();
    // 旧会话 B：更早
    log += &log_line(
        &t(0),
        "qua.start",
        &start_json("RB", "MB", "conv-b-0001", "chat"),
    );
    log += &log_line(
        &t(1000),
        "qua.end",
        &end_json("RB", "MB", "conv-b-0001", "chat", 1000, 200, 5000, 100),
    );
    // 新会话 A：更晚
    log += &log_line(
        &t(10000),
        "qua.start",
        &start_json("RA", "MA", "conv-a-0001", "chat"),
    );
    log += &log_line(
        &t(12000),
        "qua.end",
        &end_json("RA", "MA", "conv-a-0001", "chat", 2000, 300, 9000, 400),
    );
    fs::write(log_dir.join("wb.log"), log).unwrap();

    // 没有路由文件时：取最近活跃的 A
    let mut tr = Tracker::with_root(home.clone());
    tr.poll();
    let s = tr.snapshot();
    assert_eq!(s.total.unwrap().input, 9000, "应回退到最近活跃会话");

    // 路由指向 B：应跟随 B，即使 A 更晚
    let user_dir = home.join(".workbuddy/uid");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(
        user_dir.join("startup-route.json"),
        "{\"path\":\"/conversation/conv-b-0001\",\"savedAt\":1}",
    )
    .unwrap();
    let mut tr = Tracker::with_root(home.clone());
    tr.poll();
    let s = tr.snapshot();
    assert_eq!(s.total.unwrap().input, 5000, "应跟随当前打开的会话 B");
    assert_eq!(s.turns_count, 1);

    let _ = fs::remove_dir_all(&home);
}

/// 停在新任务页（用户刚点了"新对话"）时，卡片应当是空的，
/// 而不是继续显示上一个任务的数据——这是用户提的核心问题。
#[test]
fn home_page_shows_empty_state() {
    let home = tmp_home("home");
    let date = Local::now().format("%Y-%m-%d").to_string();
    let log_dir = home.join(".workbuddy/logs").join(&date);
    fs::create_dir_all(&log_dir).unwrap();

    let base = Local::now() - chrono::Duration::minutes(10);
    let t = |ms: i64| {
        (base + chrono::Duration::milliseconds(ms))
            .format("%Y/%m/%d %H:%M:%S%.3f")
            .to_string()
    };
    let mut log = String::new();
    log += &log_line(
        &t(0),
        "qua.start",
        &start_json("R1", "M1", "conv-old-01", "chat"),
    );
    log += &log_line(
        &t(3000),
        "qua.end",
        &end_json("R1", "M1", "conv-old-01", "chat", 3000, 800, 12000, 600),
    );
    fs::write(log_dir.join("wb.log"), log).unwrap();

    let user_dir = home.join(".workbuddy/uid");
    fs::create_dir_all(&user_dir).unwrap();
    // 先停在那个会话上：有数据
    fs::write(
        user_dir.join("startup-route.json"),
        "{\"path\":\"/conversation/conv-old-01\",\"savedAt\":1}",
    )
    .unwrap();
    let mut tr = Tracker::with_root(home.clone());
    tr.poll();
    assert_eq!(tr.snapshot().total.expect("旧任务有数据").input, 12000);

    // 点"新对话"：路由变成 /home，卡片必须立刻空掉
    fs::write(
        user_dir.join("startup-route.json"),
        "{\"path\":\"/home\",\"savedAt\":2}",
    )
    .unwrap();
    tr.poll();
    let s = tr.snapshot();
    assert_eq!(s.status, Status::NoData, "新任务页应显示空态");
    assert!(s.total.is_none(), "不能还挂着上一个任务的累计值");
    assert!(s.turn.is_none());
    assert!(s.current_conv.is_empty());

    // 回到旧会话：数据要能原样回来
    fs::write(
        user_dir.join("startup-route.json"),
        "{\"path\":\"/conversation/conv-old-01\",\"savedAt\":3}",
    )
    .unwrap();
    tr.poll();
    assert_eq!(tr.snapshot().total.expect("回到旧任务应恢复").input, 12000);

    let _ = fs::remove_dir_all(&home);
}

/// 日志翻不到时：先用落盘快照，再退到 transcript 历史用量
#[test]
fn falls_back_to_saved_stats_then_transcript() {
    let home = tmp_home("fallback");
    let cache_path = home.join("tokenbar-stats.json");
    let user_dir = home.join(".workbuddy/uid");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(
        user_dir.join("startup-route.json"),
        "{\"path\":\"/conversation/conv-gone-01\",\"savedAt\":1}",
    )
    .unwrap();
    fs::write(
        &cache_path,
        r#"{"version":1,"convs":{"conv-gone-01":{"turns":2,"calls":5,"model":"m","updated":1,
            "total":{"calls":5,"input":9000,"output":700,"cache":4000,"rate":44.4,
                     "ttft":800,"duration":12000,"tps":50},
            "turn":{"calls":2,"input":3000,"output":300,"cache":1500,"rate":50,
                    "ttft":700,"duration":5000,"tps":60}}}}"#,
    )
    .unwrap();

    let mut tr = Tracker::with_root_and_cache(home.clone(), Some(cache_path.clone()));
    tr.poll();
    let s = tr.snapshot();
    assert_eq!(s.status, Status::Cached);
    assert!(s.stale, "应标记为历史快照");
    assert_eq!(s.turns_count, 2);
    let total = s.total.expect("应有累计");
    assert_eq!(total.input, 9000);
    assert_eq!(total.output, 700);
    assert_eq!(s.turn.expect("快照里本轮也有").ttft_ms, 700.0);

    // 没有快照、只有 transcript 时，只能拿到 token 三项
    fs::remove_file(&cache_path).unwrap();
    fs::write(
        user_dir.join("startup-route.json"),
        "{\"path\":\"/conversation/conv-hist-02\",\"savedAt\":2}",
    )
    .unwrap();
    fs::write(
        home.join(".workbuddy/projects/fake/conv-hist-02.jsonl"),
        assistant_json("R1", "m1", 50000, 1200, 41000),
    )
    .unwrap();
    let mut tr = Tracker::with_root(home.clone());
    tr.poll();
    let s = tr.snapshot();
    assert!(s.stale);
    let total = s.total.expect("transcript 应给出累计");
    assert_eq!(total.input, 50000);
    assert_eq!(total.output, 1200);
    assert_eq!(total.cache_read, 41000);
    assert!(s.turn.is_none(), "transcript 给不出本轮时间指标");

    let _ = fs::remove_dir_all(&home);
}
