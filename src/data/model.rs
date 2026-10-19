//! 数据模型：单次调用、一轮请求、聚合结果、对外快照。

/// 一次模型调用（日志里一条 qua.end，尚未结束的只有 qua.start）
#[derive(Clone, Debug)]
pub struct Call {
    pub start_ms: i64,
    pub end_ms: i64,
    pub ttft_ms: f32,
    pub total_ms: f32,
    pub prompt: u64,
    pub completion: u64,
    pub done: bool,
}

/// 一轮请求（同一个 conversation_request_id 下的所有调用与缓存用量）
#[derive(Clone, Debug)]
pub struct Turn {
    pub conv: String,
    pub id: String,
    pub model: String,
    pub first_start: i64,
    pub last_activity: i64,
    pub calls: Vec<Call>,
    pub cache_read: u64,
    pub cache_create: u64,
    pub cache_known: bool,
}

/// 一组可显示的指标
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub calls: usize,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_rate: Option<f32>,
    pub ttft_ms: f32,
    pub duration_ms: f32,
    pub tps: f32,
}

impl Stats {
    /// 累加一次已完成的调用的用量
    pub fn count_call(&mut self, c: &Call) {
        self.calls += 1;
        self.input += c.prompt;
        self.output += c.completion;
    }

    /// 用"累计生成时长"折算输出速度（拿不到生成时长时保持 0）
    pub fn tps_from(&mut self, gen_ms: f32) {
        if gen_ms > 0.0 {
            self.tps = self.output as f32 / (gen_ms / 1000.0);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Idle,
    Generating,
    NoData,
    /// 当前任务在日志里已经查不到（超过回看窗口 / 日志被轮转），数字来自本地快照
    Cached,
}

/// 界面每帧读取的只读快照
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: Status,
    /// 任务还在跑（思考 / 工具 / 生成都算），此时圆点与胶囊用点缀色
    pub active: bool,
    pub model: String,
    pub turn: Option<Stats>,
    pub total: Option<Stats>,
    pub turns_count: usize,
    pub elapsed_ms: f32,
    pub spark: Vec<f32>,
    pub current_conv: String,
    /// 数据不是从"当前日志"现算的，而是历史快照（切换回老任务时会这样）
    pub stale: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            status: Status::NoData,
            active: false,
            model: String::new(),
            turn: None,
            total: None,
            turns_count: 0,
            elapsed_ms: 0.0,
            spark: Vec::new(),
            current_conv: String::new(),
            stale: false,
        }
    }
}

/// 某个任务最后一次被看到时的累计值，落盘到 tokenbar-stats.json
#[derive(Clone, Debug, Default)]
pub struct CachedConv {
    pub turns: usize,
    pub calls: usize,
    pub total: Stats,
    pub turn: Option<Stats>,
    pub model: String,
    pub updated: i64,
}
