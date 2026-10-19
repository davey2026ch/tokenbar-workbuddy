//! 两种输入格式的解析：
//! 1. SSE 监控日志里的 `qua.start: {...}` / `qua.end: {...}`（时间、首字、token）
//! 2. 会话 transcript 的 jsonl（缓存命中 token）
//!
//! 解析之外的事（去重、累加、落盘）都留给 Tracker。

use chrono::{Local, NaiveDateTime};
use serde_json::Value;

/// 日志里一条 chat 调用的开始 / 结束事件
#[derive(Clone, Debug, PartialEq)]
pub struct LogEvent {
    pub end: bool,
    pub ts_ms: i64,
    pub conv: String,
    pub req: String,
    pub msg: String,
    pub model: String,
    pub total_ms: f32,
    pub ttft_ms: f32,
    pub prompt: u64,
    pub completion: u64,
}

/// transcript 里一条 assistant 回复的用量
#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub msg_id: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_create: u64,
    pub model: String,
    pub request_id: Option<String>,
}

impl Usage {
    /// 四项全 0 的记录没有任何信息量
    pub fn is_empty(&self) -> bool {
        self.input + self.output + self.cache_read + self.cache_create == 0
    }
}

/// 解析一条日志行；不是 chat 调用（比如生成标题）或不完整时返回 None
pub fn log_event(line: &str) -> Option<LogEvent> {
    let (end, json) = split_log_body(line)?;
    let ts_ms = log_time(line)?;
    let v = serde_json::from_str::<Value>(json.trim_end()).ok()?;
    if str_field(&v, "request_type") != "chat" {
        return None;
    }
    let conv = str_field(&v, "conversation_id");
    let req = str_field(&v, "conversation_request_id");
    let msg = str_field(&v, "message_id");
    if conv.is_empty() || req.is_empty() || msg.is_empty() {
        return None;
    }
    Some(LogEvent {
        end,
        ts_ms,
        model: str_field(&v, "model"),
        conv,
        req,
        msg,
        total_ms: num(&v, "total_cost_ms"),
        ttft_ms: num(&v, "ttft_ms"),
        prompt: num(&v, "prompt_tokens") as u64,
        completion: num(&v, "completion_tokens") as u64,
    })
}

/// 解析一行 transcript；只认带 usage 的记录
///
/// 两种写法都要认：
/// - 老（Anthropic 风格）：`{"type":"assistant","message":{"id":…,"usage":{…}},"_requestId":…}`
/// - 新（当前版本）：`{"id":…,"type":"function_call","message":{"usage":{…}},"providerData":{…}}`
///   记录里没有 message.id，用记录自己的 id 去重；请求 id 在 providerData.conversationRequestId。
pub fn usage(line: &str) -> Option<Usage> {
    // 先做廉价的子串判断，绝大多数行不用进 JSON 解析
    if !line.contains("\"usage\"") || !line.contains("\"message\"") {
        return None;
    }
    let v = serde_json::from_str::<Value>(line).ok()?;
    let msg = v.get("message").filter(|m| m.is_object())?;
    let usage = msg.get("usage").filter(|u| u.is_object())?;
    let u = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);

    let provider = v.get("providerData");
    let cached = provider
        .and_then(|p| p.get("rawUsage"))
        .and_then(|r| r.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let msg_id = msg
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| v.get("id").and_then(Value::as_str))
        .unwrap_or("");
    if msg_id.is_empty() {
        return None;
    }
    let request_id = v
        .get("_requestId")
        .and_then(Value::as_str)
        .or_else(|| msg.get("_requestId").and_then(Value::as_str))
        .or_else(|| {
            provider
                .and_then(|p| p.get("conversationRequestId"))
                .and_then(Value::as_str)
        })
        .map(|s| s.to_string());
    Some(Usage {
        msg_id: msg_id.to_string(),
        input: u("input_tokens"),
        output: u("output_tokens"),
        cache_read: u("cache_read_input_tokens").max(cached),
        cache_create: u("cache_creation_input_tokens")
            .max(
                provider
                    .and_then(|p| p.get("rawUsage"))
                    .and_then(|r| r.get("prompt_cache_write_tokens"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            ),
        model: msg
            .get("model")
            .and_then(Value::as_str)
            .or_else(|| provider.and_then(|p| p.get("model")).and_then(Value::as_str))
            .unwrap_or("")
            .to_string(),
        request_id,
    })
}

/// 会话状态机的一条转换：返回 (时间戳, 是否在干活)。
///
/// 行形如 `2026-10-18T15:38:37.042Z state-machine:transition {"input":"PHASE_WORKING",…,"to":"working"}`
/// —— 跑工具、思考、生成时都是 working/planning，一轮真正结束才回到 idle。
pub fn conv_state(line: &str) -> Option<(i64, bool)> {
    if !line.contains("state-machine:transition") {
        return None;
    }
    let ts = line.split_whitespace().next()?;
    let ts_ms = chrono::DateTime::parse_from_rfc3339(ts)
        .ok()?
        .timestamp_millis();
    let p = line.find("\"to\":\"")? + "\"to\":\"".len();
    let rest = &line[p..];
    let state = &rest[..rest.find('"')?];
    let active = match state {
        "working" | "planning" => true,
        "idle" => false,
        _ => return None,
    };
    Some((ts_ms, active))
}

/// 日志形如 `[2026/10/18 21:00:00.123] [Info] ... qua.end: {...}`
fn split_log_body(line: &str) -> Option<(bool, &str)> {
    if let Some(p) = line.find("qua.end: {") {
        return Some((true, &line[p + "qua.end: ".len()..]));
    }
    let p = line.find("qua.start: {")?;
    Some((false, &line[p + "qua.start: ".len()..]))
}

fn log_time(line: &str) -> Option<i64> {
    let start = line.find('[')? + 1;
    let end = line[start..].find(']')? + start;
    parse_log_ts(line[start..end].trim())
}

/// 日志时间戳的写法跟平台/系统区域有关，全都认：
/// - Windows（中文/24 小时）：`2026/10/18 21:00:00.123`、`2026-10-18 21:00:00.123`
/// - macOS（英文 12 小时）：`10/19/2026, 12:38:19 AM.538`
/// - ISO（状态机日志等）：`2026-10-18T16:38:16.078Z`
pub fn parse_log_ts(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // 带时区的（Z / +08:00）按绝对时间算，别再当本地时间
    if raw.contains('T') {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
            return Some(dt.timestamp_millis());
        }
    }
    const FORMATS: [&str; 8] = [
        "%Y/%m/%d %H:%M:%S%.3f",
        "%Y-%m-%d %H:%M:%S%.3f",
        "%Y/%m/%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%m/%d/%Y, %I:%M:%S %p%.3f",
        "%m/%d/%Y, %I:%M:%S%.3f %p",
        "%m/%d/%Y, %I:%M:%S %p",
        "%m/%d/%Y %I:%M:%S %p%.3f",
    ];
    for f in FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(raw, f) {
            if let Some(dt) = naive.and_local_timezone(Local).single() {
                return Some(dt.timestamp_millis());
            }
        }
    }
    None
}

pub fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// 日志里的数字大多是字符串，这里两种都接受
pub fn num(v: &Value, key: &str) -> f32 {
    match v.get(key) {
        Some(Value::String(s)) => s.parse::<f32>().unwrap_or(0.0),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) as f32,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS: &str = "[2026/10/18 10:00:03.000] [Info] [pid=1] [ModelProvider]  [SseMonitor#01] ";

    fn end_line(extra: &str) -> String {
        format!(
            "{TS}qua.end: {{\"conversation_id\":\"c1\",\"conversation_request_id\":\"R1\",\"message_id\":\"M1\",\"model\":\"m\",\"request_type\":\"chat\"{extra}}}"
        )
    }

    #[test]
    fn parses_end_event() {
        let ev = log_event(&end_line(
            ",\"total_cost_ms\":\"3000\",\"ttft_ms\":\"1000\",\"prompt_tokens\":\"10000\",\"completion_tokens\":\"500\"",
        ))
        .expect("应解析成功");
        assert!(ev.end);
        assert_eq!(ev.conv, "c1");
        assert_eq!(ev.req, "R1");
        assert_eq!(ev.msg, "M1");
        assert!((ev.total_ms - 3000.0).abs() < 0.1);
        assert!((ev.ttft_ms - 1000.0).abs() < 0.1);
        assert_eq!(ev.prompt, 10000);
        assert_eq!(ev.completion, 500);
        assert!(ev.ts_ms > 0);
    }

    #[test]
    fn rejects_other_request_types() {
        let line = end_line("").replace("\"chat\"", "\"conversation_topic\"");
        assert!(log_event(&line).is_none(), "标题请求不该被统计");
        assert!(log_event("[2026/10/18 10:00:03.000] [Info] 随便一行").is_none());
    }

    #[test]
    fn parses_session_state() {
        let working = r#"2026-10-18T15:38:37.042Z state-machine:transition {"conversationId":"c1","input":"PHASE_WORKING","from":"planning","to":"working","actions":[]}"#;
        assert_eq!(conv_state(working), Some((1792337917042, true)));

        let idle = r#"2026-10-18T15:25:40.618Z state-machine:transition {"conversationId":"c1","input":"TURN_COMPLETED","from":"working","valid":true,"to":"idle"}"#;
        assert_eq!(conv_state(idle).map(|(_, a)| a), Some(false));

        // 连接类状态不改变"在不在干活"
        let conn = r#"2026-10-18T15:25:40.618Z state-machine:transition {"to":"connected"}"#;
        assert_eq!(conv_state(conn), None);
        assert_eq!(conv_state("随便一行"), None);
    }

    #[test]
    fn parses_usage_record() {
        let line = r#"{"type":"assistant","message":{"id":"M1","model":"m","usage":{"input_tokens":10000,"output_tokens":500,"cache_read_input_tokens":6000,"cache_creation_input_tokens":0}},"_requestId":"R1"}"#;
        let u = usage(line).expect("应解析成功");
        assert_eq!(u.msg_id, "M1");
        assert_eq!(
            (u.input, u.output, u.cache_read, u.cache_create),
            (10000, 500, 6000, 0)
        );
        assert_eq!(u.request_id.as_deref(), Some("R1"));
        assert!(!u.is_empty());

        // 用户消息、没有 id、没有 usage 的行都要忽略
        assert!(usage(r#"{"type":"user","message":{"content":"hi"}}"#).is_none());
        assert!(usage(
            r#"{"type":"assistant","message":{"usage":{"input_tokens":1,"output_tokens":1}}}"#
        )
        .is_none());
        assert!(usage("not json").is_none());
    }

    #[test]
    fn parses_current_transcript_record() {
        // 当前版本工作写入的形态：记录自己没有 message.id，缓存数字在 rawUsage 里
        let line = r#"{"id":"01a1-REC","type":"function_call","message":{"usage":{"input_tokens":34227,"output_tokens":212,"total_tokens":34439}},"providerData":{"conversationRequestId":"01a1-REQ","messageId":"01a1-MSG","model":"deepseek-v4.1-flash","rawUsage":{"prompt_tokens":34227,"completion_tokens":212,"prompt_tokens_details":{"cached_tokens":30000},"prompt_cache_write_tokens":900}}}"#;
        let u = usage(line).expect("应解析成功");
        assert_eq!(u.msg_id, "01a1-REC");
        assert_eq!((u.input, u.output), (34227, 212));
        assert_eq!(u.cache_read, 30000);
        assert_eq!(u.cache_create, 900);
        assert_eq!(u.request_id.as_deref(), Some("01a1-REQ"));
        assert_eq!(u.model, "deepseek-v4.1-flash");

        // 直接写在 usage 里的缓存字段优先于 rawUsage
        let line2 = r#"{"id":"R2","type":"message","message":{"usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":7}},"providerData":{"conversationRequestId":"Q2","rawUsage":{"prompt_tokens_details":{"cached_tokens":1}}}}"#;
        assert_eq!(usage(line2).unwrap().cache_read, 7);
    }

    #[test]
    fn parses_all_known_timestamp_shapes() {
        // Windows 24 小时制
        assert_eq!(
            parse_log_ts("2026/10/18 10:00:03.000"),
            Some(
                NaiveDateTime::parse_from_str("2026-10-18 10:00:03.000", "%Y-%m-%d %H:%M:%S%.3f")
                    .unwrap()
                    .and_local_timezone(Local)
                    .unwrap()
                    .timestamp_millis()
            )
        );
        // macOS 英文 12 小时制（当前机器上的真实写法）
        let pm = parse_log_ts("10/19/2026, 12:38:19 AM.538").expect("macOS 时间戳要认");
        let noon = parse_log_ts("10/19/2026, 12:38:19 PM.538").expect("macOS 时间戳要认");
        assert_eq!(noon - pm, 12 * 3600 * 1000, "AM/PM 必须区分开");
        // 毫秒在 AM 前面的那种写法也要认
        assert_eq!(
            parse_log_ts("10/19/2026, 12:38:19.538 AM"),
            Some(pm),
            "两种毫秒位置都要能解析"
        );
        // ISO（状态机日志）
        assert_eq!(
            parse_log_ts("2026-10-18T15:38:37.042Z"),
            Some(1792337917042)
        );
        assert_eq!(parse_log_ts("随便什么"), None);
    }
}
