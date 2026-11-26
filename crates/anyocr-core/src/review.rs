//! 视觉大模型复核：数据结构、防幻觉校验与应用。
//!
//! 复核只能改已有行的文字：不新增、不删除行，也不动坐标。每处修改先经过 [`check_edit`]，
//! 被拒绝的修改同样记入日志，便于导出核查。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use image::RgbImage;
use serde::{Deserialize, Serialize};

use crate::engine::EngineError;
use crate::types::{BlockSource, PageResult};

/// 复核器：拿页面图片和 OCR 结果，返回逐行修改建议（已做过校验）。
pub trait Reviewer: Send + Sync {
    /// 用于日志与界面的名称，如 `deepseek-flash`。
    fn name(&self) -> &str;

    fn review(&self, image: &RgbImage, page: &PageResult) -> Result<ReviewOutcome, EngineError>;
}

/// 一处修改建议。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineChange {
    /// `PageResult::blocks` 的下标。
    pub index: usize,
    pub before: String,
    pub after: String,
    pub accepted: bool,
    /// 拒绝原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 一页的复核结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReviewOutcome {
    pub changes: Vec<LineChange>,
    pub elapsed_ms: u64,
    pub model: String,
}

impl ReviewOutcome {
    pub fn accepted(&self) -> usize {
        self.changes.iter().filter(|c| c.accepted).count()
    }
    pub fn rejected(&self) -> usize {
        self.changes.len() - self.accepted()
    }
}

/// 校验参数。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Guard {
    /// 编辑距离 / 原文长度的上限，超过视为改动过大（可能是幻觉）。
    pub max_edit_ratio: f32,
    /// 原文很短时允许的最小编辑次数（例如 2 个字的行改 1 个字是 0.5）。
    pub min_edits_allowed: usize,
    /// 拒绝引入原文没有的 LaTeX / Markdown 标记（`$ \ _ ^ { }`），否则文字层搜不到。
    pub reject_markup: bool,
    /// 一页中“改成另一行原文”的修改占比超过该值时，判定为行号错位，整页修改作废。
    pub max_shift_ratio: f32,
    /// 拒绝把数字改成别的数字（位数不变、数值变化），数字错误代价高，交给人工核对。
    pub protect_digits: bool,
}

impl Default for Guard {
    fn default() -> Self {
        Self {
            max_edit_ratio: 0.5,
            min_edits_allowed: 2,
            reject_markup: true,
            max_shift_ratio: 0.3,
            protect_digits: true,
        }
    }
}

const MARKUP: [char; 6] = ['$', '\\', '_', '^', '{', '}'];

fn strip_ws(s: &str) -> Vec<char> {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 字符级编辑距离（Levenshtein）。
pub fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 检查一处修改。`Ok(None)` 表示实质无改动（只差空白），应忽略；`Ok(Some(()))` 接受；`Err` 为拒绝原因。
pub fn check_edit(before: &str, after: &str, guard: &Guard) -> Result<Option<()>, String> {
    let (a, b) = (strip_ws(before), strip_ws(after));
    if a == b {
        return Ok(None);
    }
    if b.is_empty() {
        return Err("修改后为空".into());
    }
    if guard.reject_markup
        && let Some(m) = MARKUP.iter().find(|m| b.contains(m) && !a.contains(m))
    {
        return Err(format!("引入了排版标记“{m}”"));
    }
    if guard.protect_digits {
        // 形近字母按数字计（l/I/| → 1，O/o → 0），这样“l66 → 166”“由0向外 → 由O向外”不算改动数字
        let digits = |s: &[char]| {
            s.iter()
                .filter_map(|c| match c {
                    '0'..='9' => Some(*c),
                    'l' | 'I' | '|' => Some('1'),
                    'O' | 'o' => Some('0'),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let (da, db) = (digits(&a), digits(&b));
        if db.len() > da.len() {
            // 实测模型会把看不清的下标 i 猜成 1、10
            return Err("新增了数字，需人工核对".into());
        }
        if da.len() == db.len() && da != db {
            return Err("改动了数字，需人工核对".into());
        }
    }
    let d = edit_distance(&a, &b);
    let allowed =
        ((a.len() as f32 * guard.max_edit_ratio).floor() as usize).max(guard.min_edits_allowed);
    if d > allowed {
        return Err(format!("改动过大（编辑距离 {d}，上限 {allowed}）"));
    }
    Ok(Some(()))
}

/// 校验模型返回的 `(行号, 新文本)`，生成修改日志。越界行号、重复行号与无实质改动的条目被丢弃。
///
/// 除逐行的 [`check_edit`] 外，还检查行号错位：模型把相邻行的内容填到当前行（常见于表格），
/// 表现为修改后的文字恰好等于本页另一行的原文。这类修改一律拒绝；占比过高时整页作废。
pub fn evaluate(page: &PageResult, fixes: &[(usize, String)], guard: &Guard) -> Vec<LineChange> {
    let originals: std::collections::HashMap<Vec<char>, Vec<usize>> = page
        .blocks
        .iter()
        .enumerate()
        .fold(Default::default(), |mut m, (i, b)| {
            m.entry(strip_ws(&b.text)).or_default().push(i);
            m
        });
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut shifted = 0usize;
    for (index, after) in fixes {
        let Some(block) = page.blocks.get(*index) else {
            continue;
        };
        if !seen.insert(*index) {
            continue;
        }
        let after = after.trim().to_owned();
        let copies_other = originals
            .get(&strip_ws(&after))
            .is_some_and(|ids| ids.iter().any(|i| i != index));
        let verdict = if copies_other && strip_ws(&after) != strip_ws(&block.text) {
            shifted += 1;
            Err("与本页另一行相同，疑似行号错位".to_owned())
        } else {
            check_edit(&block.text, &after, guard)
        };
        match verdict {
            Ok(None) => {}
            Ok(Some(())) => out.push(LineChange {
                index: *index,
                before: block.text.clone(),
                after,
                accepted: true,
                reason: None,
            }),
            Err(reason) => out.push(LineChange {
                index: *index,
                before: block.text.clone(),
                after,
                accepted: false,
                reason: Some(reason),
            }),
        }
    }
    if !out.is_empty() && shifted as f32 / out.len() as f32 > guard.max_shift_ratio {
        for c in out.iter_mut().filter(|c| c.accepted) {
            c.accepted = false;
            c.reason = Some("本页多处行号错位，整页修改作废".into());
        }
    }
    out
}

/// 把通过校验的修改写回页面：只替换文字，来源标记为 `Reviewed`。
/// 若页面的行已变化（`before` 与当前文字不符），跳过该条。返回实际应用的条数。
pub fn apply(page: &mut PageResult, outcome: &ReviewOutcome) -> usize {
    let mut n = 0;
    for c in outcome.changes.iter().filter(|c| c.accepted) {
        if let Some(b) = page.blocks.get_mut(c.index)
            && b.text == c.before
        {
            b.text = c.after.clone();
            b.source = BlockSource::Reviewed;
            n += 1;
        }
    }
    n
}

/// 复核设置：与 OCR 并行，按页提交。
#[derive(Clone)]
pub struct ReviewSetup {
    pub reviewer: Arc<dyn Reviewer>,
    /// 同时进行的复核请求数。
    pub concurrency: usize,
    /// OCR 全部完成后最多再等多久。
    pub grace: Duration,
    /// 设置后只复核含置信度低于该值的行的页。
    pub low_conf: Option<f32>,
}

/// 复核调度：OCR 结果一到就提交，复核线程与后续页的 OCR 并行。
pub struct ReviewPool {
    queue: Option<mpsc::Sender<(usize, RgbImage, PageResult)>>,
    results: mpsc::Receiver<(usize, Result<ReviewOutcome, String>)>,
    stop: Arc<AtomicBool>,
    low_conf: Option<f32>,
    submitted: std::collections::HashSet<usize>,
    /// 已返回的结果：页序号 → 复核结果或错误。
    pub outcomes: HashMap<usize, Result<ReviewOutcome, String>>,
}

impl ReviewPool {
    /// 启动 setup.concurrency 个复核线程。
    pub fn start(setup: &ReviewSetup) -> Self {
        let (qtx, qrx) = mpsc::channel::<(usize, RgbImage, PageResult)>();
        let (vtx, vrx) = mpsc::channel();
        let qrx = Arc::new(Mutex::new(qrx));
        let stop = Arc::new(AtomicBool::new(false));
        for _ in 0..setup.concurrency.max(1) {
            let (qrx, vtx, stop) = (Arc::clone(&qrx), vtx.clone(), Arc::clone(&stop));
            let reviewer = Arc::clone(&setup.reviewer);
            // 不 join：超过截止时间后主线程不再等待，线程处理完手头请求后自行退出
            std::thread::spawn(move || {
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let job = qrx.lock().ok().and_then(|r| r.recv().ok());
                    let Some((i, img, page)) = job else { break };
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let r = reviewer.review(&img, &page).map_err(|e| e.to_string());
                    if vtx.send((i, r)).is_err() {
                        break;
                    }
                }
            });
        }
        Self {
            queue: Some(qtx),
            results: vrx,
            stop,
            low_conf: setup.low_conf,
            submitted: Default::default(),
            outcomes: HashMap::new(),
        }
    }

    /// 提交一页；空页、或设置了低置信度阈值而本页没有低置信度行时不提交。
    pub fn submit(&mut self, index: usize, image: RgbImage, page: &PageResult) {
        let low = page.blocks.iter().map(|b| b.score).fold(1.0f32, f32::min);
        if page.blocks.is_empty() || self.low_conf.is_some_and(|t| low >= t) {
            return;
        }
        if let Some(q) = &self.queue
            && q.send((index, image, page.clone())).is_ok()
        {
            self.submitted.insert(index);
        }
    }

    /// 已提交的页数。
    pub fn submitted_count(&self) -> usize {
        self.submitted.len()
    }

    pub fn was_submitted(&self, index: usize) -> bool {
        self.submitted.contains(&index)
    }

    /// 非阻塞地收取已完成的复核结果。
    pub fn poll(&mut self) {
        while let Ok((i, r)) = self.results.try_recv() {
            self.outcomes.insert(i, r);
        }
    }

    /// OCR 全部完成后调用：最多等到 `deadline`，然后让复核线程停止领取新任务。
    pub fn finish(&mut self, deadline: Instant, cancel: &AtomicBool) {
        self.queue = None;
        while self.outcomes.len() < self.submitted.len() && !cancel.load(Ordering::Relaxed) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let wait = (deadline - now).min(Duration::from_millis(200));
            if let Ok((i, r)) = self.results.recv_timeout(wait) {
                self.outcomes.insert(i, r);
            }
        }
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TextBlock;

    fn page(lines: &[&str]) -> PageResult {
        PageResult {
            blocks: lines
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    TextBlock::from_rect(
                        *t,
                        [0.0, i as f32 * 10.0, 100.0, i as f32 * 10.0 + 8.0],
                        0.9,
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn distance() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(edit_distance(&c("干沙"), &c("干涉")), 1);
        assert_eq!(edit_distance(&c(""), &c("abc")), 3);
        assert_eq!(edit_distance(&c("kitten"), &c("sitting")), 3);
    }

    #[test]
    fn guard_rules() {
        let g = Guard::default();
        assert_eq!(
            check_edit("实验3.19光的等厚干沙", "实验 3.19 光的等厚干涉", &g),
            Ok(Some(()))
        );
        // 只差空白：忽略
        assert_eq!(check_edit("实验3.19", "实验 3.19", &g), Ok(None));
        assert!(check_edit("光的干涉", "", &g).is_err());
        // 整行换成别的内容：拒绝
        assert!(check_edit("当频率相同的两束光波相遇时", "本实验研究牛顿环", &g).is_err());
        // 很短的行允许改 2 个字
        assert_eq!(check_edit("l66", "166", &g), Ok(Some(())));
        // 引入 LaTeX 标记：拒绝；原文已有则允许
        assert!(check_edit("r第k级明环的半径", "$r_k$第k级明环的半径", &g).is_err());
        // 数字：位数不变而数值变化的拒绝；补漏、字母认成数字的允许
        assert!(check_edit("单位10-2mm", "单位10-3mm", &g).is_err());
        assert_eq!(check_edit("l66", "166", &g), Ok(Some(())));
        assert!(check_edit("xi", "x1", &g).is_err());
        assert!(check_edit("记下读数x，再沿同方向", "记下读数x1，再沿同方向", &g).is_err());
        assert_eq!(
            check_edit("空气层由0向外", "空气层由O向外", &g),
            Ok(Some(()))
        );
        // 上标数字替换普通数字（数字变少）允许
        assert_eq!(
            check_edit("R = √ri +(R - ek)2", "R = √rk² +(R - ek)²", &g),
            Ok(Some(()))
        );
        assert_eq!(check_edit("x_1 = 2", "x_1 = 2.", &g), Ok(Some(())));
    }

    #[test]
    fn detects_row_shift() {
        // 表格：模型把下一行的内容填到当前行
        let p = page(&["15", "14", "13", "12", "左", "右", "牛頓环"]);
        let fixes = vec![
            (0, "14".to_owned()),
            (1, "13".to_owned()),
            (4, "右".to_owned()),
            (6, "牛顿环".to_owned()),
        ];
        let changes = evaluate(&p, &fixes, &Guard::default());
        // 错位占 3/4，超过阈值：整页作废，连正确的“牛顿环”也不采纳
        assert!(changes.iter().all(|c| !c.accepted));
        // 只有少量错位时，其余正确修改照常采纳
        let fixes = vec![
            (0, "14".to_owned()),
            (6, "牛顿环".to_owned()),
            (2, "18".to_owned()),
            (3, "19".to_owned()),
        ];
        let g = Guard {
            protect_digits: false,
            ..Guard::default()
        };
        let changes = evaluate(&p, &fixes, &g);
        let ok: Vec<_> = changes
            .iter()
            .filter(|c| c.accepted)
            .map(|c| c.index)
            .collect();
        assert_eq!(ok, vec![6, 2, 3]);
    }

    #[test]
    fn evaluate_and_apply() {
        let mut p = page(&["光的千涉", "166", "正常行"]);
        let fixes = vec![
            (0, "光的干涉".to_owned()),
            (0, "重复".to_owned()),
            (1, "完全不同的一整行内容".to_owned()),
            (9, "越界".to_owned()),
            (2, "正常 行".to_owned()),
        ];
        let changes = evaluate(&p, &fixes, &Guard::default());
        assert_eq!(changes.len(), 2);
        assert!(changes[0].accepted);
        assert!(!changes[1].accepted);
        let outcome = ReviewOutcome {
            changes,
            ..Default::default()
        };
        assert_eq!(apply(&mut p, &outcome), 1);
        assert_eq!(p.blocks[0].text, "光的干涉");
        assert_eq!(p.blocks[0].source, BlockSource::Reviewed);
        assert_eq!(p.blocks[1].text, "166");
    }
}
