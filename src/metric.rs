//! 指标定义：键名、标签、数值格式化、配色语义。
//!
//! 配置读写、横排/竖排绘制、设置窗口的指标列表都从这里取定义，
//! 避免"同一批指标在三个地方各写一遍字符串"。

use crate::data::Stats;

/// 六个可显示的指标
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Metric {
    Ttft,
    Duration,
    Tps,
    Input,
    Output,
    Cache,
}

impl Metric {
    /// 默认顺序（也是 ini 里 order 的默认值）
    pub const ALL: [Metric; 6] = [
        Metric::Ttft,
        Metric::Duration,
        Metric::Tps,
        Metric::Input,
        Metric::Output,
        Metric::Cache,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Metric::Ttft => "ttft",
            Metric::Duration => "duration",
            Metric::Tps => "tps",
            Metric::Input => "input",
            Metric::Output => "output",
            Metric::Cache => "cache",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }

    /// 在 `Metric::ALL` 里的下标（配置里用数组存开关）
    pub fn index(self) -> usize {
        self as usize
    }

    /// bar 上的短标签
    pub fn short_label(self) -> &'static str {
        match self {
            Metric::Ttft => "首字",
            Metric::Duration => "耗时",
            Metric::Tps => "速度",
            Metric::Input => "输入",
            Metric::Output => "输出",
            Metric::Cache => "缓存",
        }
    }

    /// 设置窗口里的完整标签
    pub fn full_label(self) -> &'static str {
        match self {
            Metric::Ttft => "首字延迟",
            Metric::Duration => "响应耗时",
            Metric::Tps => "输出速度",
            Metric::Input => "输入 Tokens",
            Metric::Output => "输出 Tokens",
            Metric::Cache => "缓存命中",
        }
    }

    /// `known == false` 表示这一行根本没有数据（新会话），输入/输出显示破折号
    pub fn value(self, st: &Stats, known: bool) -> String {
        match self {
            Metric::Ttft => fmt_ms(st.ttft_ms),
            Metric::Duration => fmt_ms(st.duration_ms),
            Metric::Tps => fmt_rate(st.tps),
            Metric::Input if known => fmt_tokens(st.input),
            Metric::Output if known => fmt_tokens(st.output),
            Metric::Input | Metric::Output => "—".into(),
            Metric::Cache if st.cache_read == 0 => "—".into(),
            Metric::Cache => fmt_tokens(st.cache_read),
        }
    }

    /// 数值后面的小字（目前只有缓存命中率）
    pub fn suffix(self, st: &Stats) -> String {
        match self {
            Metric::Cache => st
                .cache_rate
                .map(|r| format!("·{:.0}%", r))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn tone(self, st: &Stats) -> Tone {
        match self {
            Metric::Ttft if st.ttft_ms > 3000.0 => Tone::Warm,
            Metric::Tps => Tone::Accent,
            Metric::Cache if st.cache_read == 0 => Tone::Faint,
            Metric::Cache => Tone::Cache,
            _ => Tone::Normal,
        }
    }
}

/// 数值配色语义，具体颜色交给 ui 层的调色板
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Normal,
    Accent,
    Cache,
    Faint,
    Warm,
}

pub fn fmt_ms(ms: f32) -> String {
    if ms < 1.0 {
        "—".into()
    } else if ms < 1000.0 {
        format!("{}ms", ms.round() as i64)
    } else if ms < 10_000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else if ms < 60_000.0 {
        format!("{:.1}s", ms / 1000.0)
    } else {
        let s = (ms / 1000.0).round() as i64;
        if s < 3600 {
            format!("{}m{:02}s", s / 60, s % 60)
        } else {
            format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
        }
    }
}

pub fn fmt_tokens(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 1_000_000 {
        let v = n as f32 / 1000.0;
        if v < 10.0 {
            format!("{:.2}K", v)
        } else if v < 100.0 {
            format!("{:.1}K", v)
        } else {
            format!("{:.0}K", v)
        }
    } else {
        format!("{:.2}M", n as f32 / 1_000_000.0)
    }
}

pub fn fmt_rate(r: f32) -> String {
    if r <= 0.0 {
        "—".into()
    } else if r < 10.0 {
        format!("{:.1}/s", r)
    } else {
        format!("{:.0}/s", r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        for (i, m) in Metric::ALL.into_iter().enumerate() {
            assert_eq!(m.index(), i, "下标必须与 ALL 的顺序一致");
            assert_eq!(Metric::from_key(m.key()), Some(m), "key={}", m.key());
            assert!(!m.short_label().is_empty());
            assert!(!m.full_label().is_empty());
        }
        assert_eq!(Metric::from_key("nope"), None);
    }

    #[test]
    fn formats_durations() {
        assert_eq!(fmt_ms(0.0), "—");
        assert_eq!(fmt_ms(800.0), "800ms");
        assert_eq!(fmt_ms(1400.0), "1.40s");
        assert_eq!(fmt_ms(7400.0), "7.40s");
        assert_eq!(fmt_ms(65_000.0), "1m05s");
        assert_eq!(fmt_ms(3_900_000.0), "1h05m");
    }

    #[test]
    fn formats_tokens_and_rates() {
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1000), "1.00K");
        assert_eq!(fmt_tokens(13_000), "13.0K");
        assert_eq!(fmt_tokens(803_725), "804K");
        assert_eq!(fmt_tokens(1_500_000), "1.50M");
        assert_eq!(fmt_rate(0.0), "—");
        assert_eq!(fmt_rate(9.5), "9.5/s");
        assert_eq!(fmt_rate(333.3), "333/s");
    }

    #[test]
    fn cache_chip_shows_rate_suffix() {
        let st = Stats {
            cache_read: 4000,
            cache_rate: Some(30.77),
            ..Default::default()
        };
        assert_eq!(Metric::Cache.value(&st, true), "4.00K");
        assert_eq!(Metric::Cache.suffix(&st), "·31%");
        assert_eq!(Metric::Cache.tone(&st), Tone::Cache);

        let empty = Stats::default();
        assert_eq!(Metric::Cache.value(&empty, true), "—");
        assert_eq!(Metric::Cache.suffix(&empty), "");
        assert_eq!(Metric::Cache.tone(&empty), Tone::Faint);

        // 新会话：输入/输出没有数据时不该显示 0
        assert_eq!(Metric::Input.value(&empty, false), "—");
        assert_eq!(Metric::Input.value(&empty, true), "0");
    }
}
