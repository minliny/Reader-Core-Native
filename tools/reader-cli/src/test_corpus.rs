//! `--test-corpus` — 批量跑 corpus-manifest.json 中的所有源.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::test_source::{test_source, SourceTestResult, StepStatus, TestSourceConfig};

/// 批量测试配置.
pub struct TestCorpusConfig {
    pub manifest_path: PathBuf,
    pub keyword: String,
    pub out_path: Option<PathBuf>,
    pub max_sources: Option<usize>,
    pub timeout: Duration,
    pub priority: Option<String>,
    pub concurrency: usize, // 当前只支持 1(串行);>1 标记为 TODO
    pub record_dir: Option<PathBuf>,
    pub offline_dir: Option<PathBuf>,
    /// 详细日志(单源级别)
    pub verbose: bool,
    /// 每完成 N 个源保存一次中间结果(防止崩溃丢失)
    pub save_interval: usize,
}

impl Default for TestCorpusConfig {
    fn default() -> Self {
        Self {
            manifest_path: PathBuf::new(),
            keyword: String::new(),
            out_path: None,
            max_sources: None,
            timeout: Duration::from_secs(15),
            priority: None,
            concurrency: 1,
            record_dir: None,
            offline_dir: None,
            verbose: false,
            save_interval: 10,
        }
    }
}

/// 批量结果汇总.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorpusBatchResult {
    pub version: String,
    pub mode: String, // "live" | "offline"
    pub keyword: String,
    pub generated_at: String,
    pub total: usize,
    pub summary: BatchSummary,
    pub sources: Vec<SourceTestResult>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BatchSummary {
    pub fully_passed: usize,
    pub partially_passed: usize,
    pub fully_failed: usize,
    pub pass_rate: f64,
    pub by_level: BTreeMap<String, LevelTally>,
    pub by_form: BTreeMap<String, FormTally>,
    pub by_priority: BTreeMap<String, FormTally>,
    pub failure_reasons: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LevelTally {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FormTally {
    pub passed: usize,
    pub failed: usize,
}

/// 跑批量测试.
pub fn run_test_corpus(config: &TestCorpusConfig) -> Result<CorpusBatchResult, String> {
    // 读 manifest
    let manifest_raw = fs::read_to_string(&config.manifest_path)
        .map_err(|err| format!("failed to read manifest: {err}"))?;
    let manifest: Value = serde_json::from_str(&manifest_raw)
        .map_err(|err| format!("failed to parse manifest: {err}"))?;
    let sources = manifest
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| "manifest missing `sources` array".to_string())?;

    // 过滤 + 限制
    let mut filtered: Vec<&Value> = sources
        .iter()
        .filter(|s| {
            if let Some(p) = &config.priority {
                s.get("priority").and_then(Value::as_str) == Some(p.as_str())
            } else {
                true
            }
        })
        .collect();
    if let Some(max) = config.max_sources {
        filtered.truncate(max);
    }

    let total = filtered.len();
    let sources_dir = config
        .manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("sources");
    let mode = if config.offline_dir.is_some() {
        "offline"
    } else {
        "live"
    };

    eprintln!(
        "test-corpus [{mode}]: {total} sources (priority={:?}, concurrency={})",
        config.priority, config.concurrency
    );

    let mut results: Vec<SourceTestResult> = Vec::with_capacity(total);
    let start = Instant::now();
    let save_interval = config.save_interval.max(1);

    for (idx, src_meta) in filtered.iter().enumerate() {
        let file = src_meta.get("file").and_then(Value::as_str).unwrap_or("");
        let source_path = sources_dir.join(file);
        let _source_id = src_meta.get("id").and_then(Value::as_str).unwrap_or("");
        let _source_name = src_meta
            .get("book_source_name")
            .and_then(Value::as_str)
            .unwrap_or("");

        let mut test_config = TestSourceConfig {
            source_path,
            keyword: config.keyword.clone(),
            timeout: config.timeout,
            record_dir: config.record_dir.clone(),
            offline_dir: config.offline_dir.clone(),
            quiet: true,
        verbose: config.verbose,
        };

        // Panic 隔离:每个源在 catch_unwind 中跑,防止某个源 panic 导致全批崩溃
        let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            test_source(&test_config)
        })) {
            Ok(r) => r,
            Err(panic_payload) => {
                let panic_msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                eprintln!("  ⚠ PANIC at source {idx}: {panic_msg}");
                let mut panic_result = SourceTestResult {
                    source_id: src_meta.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                    source_file: file.to_string(),
                    source_name: src_meta.get("book_source_name").and_then(Value::as_str).unwrap_or("").to_string(),
                    source_url: src_meta.get("book_source_url").and_then(Value::as_str).unwrap_or("").to_string(),
                    priority: None,
                    rule_forms: vec![],
                    has_js: false,
                    has_multirule: false,
                    has_regex: false,
                    levels: {
                        let mut m = BTreeMap::new();
                        m.insert("L1-import".to_string(), crate::test_source::LevelResult::fail_with("panic", panic_msg.clone()));
                        m
                    },
                    failure_reason: Some("panic".to_string()),
                    duration_ms: 0,
                };
                // Mark remaining levels as skip
                for level in &["L2-search", "L3-detail", "L4-toc", "L5-content"] {
                    panic_result.levels.insert(level.to_string(), crate::test_source::LevelResult::skip("panic_in_previous_step"));
                }
                panic_result
            }
        };
        let mut result = result;
        // 把 manifest 里的元数据补回结果(priority/rule_forms/has_*)
        result.priority = src_meta
            .get("priority")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        if let Some(forms) = src_meta.get("rule_forms").and_then(Value::as_array) {
            result.rule_forms = forms
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
        result.has_js = src_meta
            .get("has_js")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        result.has_multirule = src_meta
            .get("has_multirule")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        result.has_regex = src_meta
            .get("has_regex")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // 进度打印到 stderr
        let progress = format_progress(idx + 1, total, &result);
        eprintln!("{progress}");

        results.push(result);

        // 中间结果保存:每 save_interval 个源写一次
        if let Some(out) = &config.out_path {
            if (idx + 1) % save_interval == 0 || idx + 1 == total {
                let elapsed = start.elapsed().as_secs_f64();
                let intermediate = CorpusBatchResult {
                    version: "corpus-batch-result/1".into(),
                    mode: mode.into(),
                    keyword: config.keyword.clone(),
                    generated_at: now_iso(),
                    total: idx + 1,
                    summary: aggregate(&results),
                    sources: results.clone(),
                };
                let _ = fs::write(
                    out,
                    serde_json::to_string_pretty(&intermediate).unwrap_or_default(),
                );
                eprintln!("  [checkpoint] {}/{} sources saved ({:.1}s elapsed)", idx + 1, total, elapsed);
            }
        }

        // 防止 borrow 问题
        let _ = &mut test_config;
    }

    let elapsed = start.elapsed();
    let summary = aggregate(&results);
    eprintln!(
        "test-corpus done in {:.1}s — fully_passed={}, partially_passed={}, fully_failed={}",
        elapsed.as_secs_f64(),
        summary.fully_passed,
        summary.partially_passed,
        summary.fully_failed
    );

    let batch = CorpusBatchResult {
        version: "corpus-batch-result/1".into(),
        mode: mode.into(),
        keyword: config.keyword.clone(),
        generated_at: now_iso(),
        total,
        summary,
        sources: results,
    };

    // 写出
    if let Some(out) = &config.out_path {
        if let Some(parent) = out.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(
            out,
            serde_json::to_string_pretty(&batch).unwrap_or_default(),
        )
        .map_err(|err| format!("failed to write output: {err}"))?;
    }

    Ok(batch)
}

fn format_progress(idx: usize, total: usize, result: &SourceTestResult) -> String {
    let mut marks = String::with_capacity(20);
    for level in [
        "L1-import",
        "L2-search",
        "L3-detail",
        "L4-toc",
        "L5-content",
    ] {
        let mark = match result.levels.get(level) {
            None => '⏭',
            Some(lr) => match lr.status {
                StepStatus::Pass => '✅',
                StepStatus::Fail => '❌',
                StepStatus::Skip => '⏭',
            },
        };
        marks.push(mark);
    }
    let name = if result.source_name.is_empty() {
        result.source_file.clone()
    } else {
        result.source_name.clone()
    };
    let reason = result.failure_reason.as_deref().unwrap_or("");
    format!("[{idx}/{total}] {} {marks} {reason}", truncate(&name, 30))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}…")
    }
}

fn aggregate(results: &[SourceTestResult]) -> BatchSummary {
    let mut summary = BatchSummary::default();
    let levels = [
        "L1-import",
        "L2-search",
        "L3-detail",
        "L4-toc",
        "L5-content",
    ];
    for level in &levels {
        summary.by_level.entry((*level).to_string()).or_default();
    }

    for r in results {
        // 全过 / 部分过 / 全失败
        let pass_count = r
            .levels
            .values()
            .filter(|lr| lr.status == StepStatus::Pass)
            .count();
        let fail_count = r
            .levels
            .values()
            .filter(|lr| lr.status == StepStatus::Fail)
            .count();

        if fail_count == 0 && pass_count == 5 {
            summary.fully_passed += 1;
        } else if pass_count > 0 {
            summary.partially_passed += 1;
        } else {
            summary.fully_failed += 1;
        }

        // 按 level 统计
        for level in &levels {
            let tally = summary.by_level.entry((*level).to_string()).or_default();
            match r.levels.get(*level) {
                Some(lr) => match lr.status {
                    StepStatus::Pass => tally.passed += 1,
                    StepStatus::Fail => tally.failed += 1,
                    StepStatus::Skip => tally.skipped += 1,
                },
                None => tally.skipped += 1,
            }
        }

        // 按 form 统计(只看 pass/fail 整体)
        let overall_pass = fail_count == 0 && pass_count == 5;
        for form in &r.rule_forms {
            let tally = summary.by_form.entry((*form).clone()).or_default();
            if overall_pass {
                tally.passed += 1;
            } else {
                tally.failed += 1;
            }
        }
        // by_priority
        if let Some(p) = &r.priority {
            let tally = summary.by_priority.entry(p.clone()).or_default();
            if overall_pass {
                tally.passed += 1;
            } else {
                tally.failed += 1;
            }
        }

        // failure_reason
        if let Some(reason) = &r.failure_reason {
            *summary.failure_reasons.entry(reason.clone()).or_insert(0) += 1;
        }
    }

    let total = results.len();
    summary.pass_rate = if total == 0 {
        0.0
    } else {
        summary.fully_passed as f64 / total as f64
    };

    summary
}

fn now_iso() -> String {
    // 不引入 chrono,用 SystemTime 估算 ISO 8601(只到秒,UTC)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // 简单估算:从 1970-01-01 开始
    let secs = now % 60;
    let mins = (now / 60) % 60;
    let hours = (now / 3600) % 24;
    let days = now / 86400;
    // 估算年月日(粗略,够用)
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{secs:02}Z")
}

fn days_to_ymd(days_since_epoch: u64) -> (u32, u32, u32) {
    // 简化算法:从 1970 开始,不考虑闰秒
    let mut year = 1970u32;
    let mut remaining = days_since_epoch;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if leap { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        month += 1;
    }
    let day = (remaining + 1) as u32;
    (year, month, day)
}
