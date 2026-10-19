//! 增量读取：只读文件新增部分，按行切开；末尾没写完的半行留到下次。

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

struct Tail {
    offset: u64,
    leftover: String,
}

impl Tail {
    fn new() -> Self {
        Self {
            offset: 0,
            leftover: String::new(),
        }
    }
}

/// 每个文件一份读取进度（文件被截断 / 轮转时自动重头读）
#[derive(Default)]
pub struct Tails {
    inner: std::collections::HashMap<std::path::PathBuf, Tail>,
}

impl Tails {
    /// 取走这个文件自上次以来新增的完整行
    pub fn read_new(&mut self, path: &Path) -> Vec<String> {
        let tail = self
            .inner
            .entry(path.to_path_buf())
            .or_insert_with(Tail::new);
        read_new_lines(path, tail)
    }
}

fn read_new_lines(path: &Path, tail: &mut Tail) -> Vec<String> {
    let Ok(meta) = fs::metadata(path) else {
        return Vec::new();
    };
    if meta.len() < tail.offset {
        tail.offset = 0;
        tail.leftover.clear();
    }
    if meta.len() == tail.offset {
        return Vec::new();
    }
    let Ok(mut f) = File::open(path) else {
        return Vec::new();
    };
    if f.seek(SeekFrom::Start(tail.offset)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.take(meta.len() - tail.offset)
        .read_to_end(&mut buf)
        .is_err()
    {
        return Vec::new();
    }
    tail.offset = meta.len();
    let text = format!("{}{}", tail.leftover, String::from_utf8_lossy(&buf));
    let mut lines = Vec::new();
    let mut last = 0usize;
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            let mut l = &text[last..i];
            if l.ends_with('\r') {
                l = &l[..l.len() - 1];
            }
            lines.push(l.to_string());
            last = i + 1;
        }
    }
    tail.leftover = text[last..].to_string();
    lines
}
