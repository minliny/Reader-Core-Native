#!/usr/bin/env python3
"""corpus_manager.py — Legado 书源 corpus 管理工具.

子命令:
  import    — 导入 Legado 书源集合,拆分为独立 JSON 文件
  classify  — 分析每个源的规则形态,生成分类索引 (corpus-manifest.json)
  sanitize  — 扫描脱敏 (token/cookie/apikey)
  validate  — 验证 corpus 完整性
  report    — 汇总评估看板 (evidence + batch + blockers → assessment.md)

用法示例:
  python3 tools/corpus-manager/corpus_manager.py import \
    --from /path/to/sources.json \
    --to tests/fixtures/corpus/sources/

  python3 tools/corpus-manager/corpus_manager.py classify \
    --sources tests/fixtures/corpus/sources/ \
    --out tests/fixtures/corpus/corpus-manifest.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

# ---------------------------------------------------------------------------
# 规则形态检测
# ---------------------------------------------------------------------------

# Legado DSL 前缀 → 形态标签
_PREFIX_MAP = [
    ("@css:", "css-explicit"),
    ("@CSS:", "css-explicit"),
    ("@xpath:", "xpath"),
    ("@XPath:", "xpath"),
    ("@@", "xpath"),
    ("@json:", "json-jsonpath"),
    ("@Json:", "json-jsonpath"),
    ("@js:", "js"),
    ("@JS:", "js"),
]

# 正则: 不带前缀但可识别的形态
_RE_JSONPATH = re.compile(r"^\$[\.\[]")
_RE_CSS_CLASS = re.compile(r"^[.#]?[a-zA-Z][\w-]*\.")  # .class / tag.class
# @text / @href / @textNodes 等 CSS 属性简写;排除 @js: @css: @xpath: @json: @@ @put: @get: 等已知前缀
_RE_CSS_AT = re.compile(r"^@(?!js:|JS:|css:|CSS:|xpath:|XPath:|json:|Json:|@|put:|get:)")
_RE_TEMPLATE = re.compile(r"\{\{.*?\}\}")
_RE_INLINE_JS = re.compile(r"<js>.*?</js>", re.DOTALL)
_RE_REGEX_SUFFIX = re.compile(r"##.+?##")
_RE_MULTIRULE = re.compile(r"[&|]{2}|%%")  # && || %%


def detect_rule_forms(rule_str: str) -> set[str]:
    """分析单条规则字符串,返回其包含的形态标签集合."""
    forms: set[str] = set()
    if not rule_str or not isinstance(rule_str, str):
        return forms

    # 前缀检测
    for prefix, tag in _PREFIX_MAP:
        if rule_str.startswith(prefix):
            forms.add(tag)
            # 前缀之后仍可能含其他形态,继续检测

    # JSONPath ($. / $[)
    if _RE_JSONPATH.search(rule_str) or "$." in rule_str:
        forms.add("json-jsonpath")

    # @text/@href 等 CSS 属性选择器
    if _RE_CSS_AT.match(rule_str):
        forms.add("css-shorthand")

    # {{}} 模板
    if _RE_TEMPLATE.search(rule_str):
        forms.add("template")

    # <js>...</js> 内联 JS
    if _RE_INLINE_JS.search(rule_str):
        forms.add("inline-js")

    ## regex ## replacement
    if _RE_REGEX_SUFFIX.search(rule_str):
        forms.add("regex-suffix")

    # && || %% MultiRule
    if _RE_MULTIRULE.search(rule_str):
        forms.add("multirule")

    # @put / @get 变量
    if "@put" in rule_str or "@get" in rule_str:
        forms.add("put-get")

    # 如果没有任何前缀且看起来像 CSS class/tag 选择器
    if not forms and _RE_CSS_CLASS.match(rule_str):
        forms.add("css-shorthand")

    # 如果还是空,标记为 plain (纯文本/简单字段名)
    if not forms:
        forms.add("plain")

    return forms


def classify_source(source: dict[str, Any]) -> dict[str, Any]:
    """分析一个书源 JSON,返回分类元数据."""
    all_forms: set[str] = set()
    rule_sections = [
        "ruleSearch", "ruleBookInfo", "ruleToc", "ruleContent", "ruleExplore",
    ]

    for section in rule_sections:
        section_val = source.get(section, {})
        if isinstance(section_val, dict):
            for field, val in section_val.items():
                if isinstance(val, str):
                    all_forms.update(detect_rule_forms(val))

    has_js = "js" in all_forms or "inline-js" in all_forms
    has_multirule = "multirule" in all_forms
    has_login = bool(source.get("loginUrl")) or bool(source.get("loginUi"))

    # 搜索 URL 分析
    search_url = source.get("searchUrl", "")
    has_js_search = "@js:" in search_url if isinstance(search_url, str) else False

    return {
        "rule_forms": sorted(all_forms),
        "has_js": has_js or has_js_search,
        "has_multirule": has_multirule,
        "has_login": has_login,
        "has_regex": "regex-suffix" in all_forms,
        "has_put_get": "put-get" in all_forms,
        "has_template": "template" in all_forms,
    }


def assign_priority(forms: set[str], has_js: bool, has_multirule: bool) -> str:
    """按规则形态组合分配测试优先级.

    P0: 覆盖独特规则形态组合(每种组合选代表)
    P1: 常见组合
    P2: 长尾
    """
    # 含 regex / put-get / xpath 的优先(这些是 edge case)
    if "regex-suffix" in forms or "put-get" in forms or "xpath" in forms:
        return "P0"
    # 纯 CSS shorthand + plain 是最常见
    if forms <= {"css-shorthand", "plain"} and not has_js and not has_multirule:
        return "P1"
    # MultiRule 是当前 blocker,优先
    if has_multirule:
        return "P0"
    return "P1"


# ---------------------------------------------------------------------------
# 脱敏
# ---------------------------------------------------------------------------

SENSITIVE_PATTERNS = [
    (re.compile(r"token=[a-f0-9]{16,}", re.IGNORECASE), "token=<REDACTED>"),
    (re.compile(r"cookie:\s*[\w=/+; -]+", re.IGNORECASE), "cookie: <REDACTED>"),
    (re.compile(r"apikey=[\w-]+", re.IGNORECASE), "apikey=<REDACTED>"),
    (re.compile(r"Bearer\s+[\w.-]+"), "Bearer <REDACTED>"),
    (re.compile(r"password=[\w!@#$%^&*]+", re.IGNORECASE), "password=<REDACTED>"),
]


def sanitize_value(val: str) -> tuple[str, list[str]]:
    """脱敏单个字符串,返回 (脱敏后, 脱敏字段列表)."""
    redactions = []
    result = val
    for pattern, replacement in SENSITIVE_PATTERNS:
        if pattern.search(result):
            count = len(pattern.findall(result))
            result = pattern.sub(replacement, result)
            redactions.append(f"{pattern.pattern[:30]}... ×{count}")
    return result, redactions


def sanitize_source(source: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    """递归脱敏书源 JSON."""
    all_redactions: list[str] = []

    def walk(obj):
        nonlocal all_redactions
        if isinstance(obj, dict):
            return {k: walk(v) for k, v in obj.items()}
        elif isinstance(obj, list):
            return [walk(v) for v in obj]
        elif isinstance(obj, str):
            cleaned, reds = sanitize_value(obj)
            all_redactions.extend(reds)
            return cleaned
        return obj

    cleaned = walk(source)
    return cleaned, all_redactions


# ---------------------------------------------------------------------------
# 子命令: import
# ---------------------------------------------------------------------------

def cmd_import(args):
    """导入 Legado 书源集合,拆分为独立 JSON 文件."""
    src_path = Path(args.from_file)
    out_dir = Path(args.to)
    out_dir.mkdir(parents=True, exist_ok=True)

    raw = json.loads(src_path.read_text(encoding="utf-8"))
    if not isinstance(raw, list):
        print(f"ERROR: expected JSON array, got {type(raw).__name__}", file=sys.stderr)
        return 1

    imported = 0
    skipped = 0
    for i, source in enumerate(raw):
        name = source.get("bookSourceName", f"unknown-{i}")
        url = source.get("bookSourceUrl", f"unknown-{i}")
        source_id = hashlib.md5(f"{url}|{name}".encode()).hexdigest()[:12]
        source["sourceId"] = f"corpus-{source_id}"

        # 脱敏
        if not args.no_sanitize:
            source, _ = sanitize_source(source)

        filename = f"src-{i:03d}-{source_id}.json"
        out_path = out_dir / filename
        out_path.write_text(
            json.dumps(source, ensure_ascii=False, indent=2),
            encoding="utf-8",
        )
        imported += 1

    print(f"imported {imported} sources (skipped {skipped}) to {out_dir}")
    return 0


# ---------------------------------------------------------------------------
# 子命令: classify
# ---------------------------------------------------------------------------

def cmd_classify(args):
    """分析每个源的规则形态,生成 corpus-manifest.json."""
    sources_dir = Path(args.sources)
    out_path = Path(args.out)

    source_files = sorted(sources_dir.glob("src-*.json"))
    if not source_files:
        print(f"ERROR: no src-*.json found in {sources_dir}", file=sys.stderr)
        return 1

    sources_meta = []
    by_form: dict[str, int] = {}
    by_priority = {"P0": 0, "P1": 0, "P2": 0}

    for fpath in source_files:
        source = json.loads(fpath.read_text(encoding="utf-8"))
        meta = classify_source(source)
        forms_set = set(meta["rule_forms"])
        priority = assign_priority(
            forms_set, meta["has_js"], meta["has_multirule"]
        )
        meta["priority"] = priority

        source_id = source.get("sourceId", fpath.stem)
        name = source.get("bookSourceName", "")
        url = source.get("bookSourceUrl", "")
        source_type = source.get("bookSourceType", 0)

        entry = {
            "id": source_id,
            "file": fpath.name,
            "book_source_name": name,
            "book_source_url": url,
            "book_source_type": source_type,
            **meta,
            "priority": priority,
        }
        sources_meta.append(entry)

        for form in meta["rule_forms"]:
            by_form[form] = by_form.get(form, 0) + 1
        by_priority[priority] = by_priority.get(priority, 0) + 1

    # P0 限流: 每种形态组合最多 5 个,超出的降级 P1
    from collections import defaultdict
    form_combo_seen: dict[str, int] = defaultdict(int)
    for entry in sources_meta:
        combo = ",".join(entry["rule_forms"])
        form_combo_seen[combo] += 1
        if form_combo_seen[combo] > 5 and entry["priority"] == "P0":
            entry["priority"] = "P1"
            by_priority["P0"] -= 1
            by_priority["P1"] += 1

    manifest = {
        "version": "corpus-manifest/1",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "total_sources": len(sources_meta),
        "sources": sources_meta,
        "by_form": dict(sorted(by_form.items(), key=lambda x: -x[1])),
        "by_priority": by_priority,
    }

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )

    print(f"classified {len(sources_meta)} sources → {out_path}")
    print(f"  by_form: {dict(sorted(by_form.items(), key=lambda x: -x[1]))}")
    print(f"  by_priority: {by_priority}")
    return 0


# ---------------------------------------------------------------------------
# 子命令: sanitize
# ---------------------------------------------------------------------------

def cmd_sanitize(args):
    """扫描脱敏报告."""
    sources_dir = Path(args.sources)
    report_path = Path(args.report)

    entries = []
    for fpath in sorted(sources_dir.glob("src-*.json")):
        source = json.loads(fpath.read_text(encoding="utf-8"))
        _, redactions = sanitize_source(source)
        if redactions:
            entries.append({
                "file": fpath.name,
                "source_name": source.get("bookSourceName", ""),
                "redactions": redactions,
            })

    report = {
        "version": "sanitize-report/1",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "total_scanned": len(list(sources_dir.glob("src-*.json"))),
        "total_with_sensitive": len(entries),
        "entries": entries,
    }

    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    print(f"sanitize: {len(entries)} sources had sensitive data → {report_path}")
    return 0


# ---------------------------------------------------------------------------
# 子命令: validate
# ---------------------------------------------------------------------------

def cmd_validate(args):
    """验证 corpus 完整性."""
    manifest_path = Path(args.manifest)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    errors = []
    sources_dir = manifest_path.parent / "sources"

    for entry in manifest["sources"]:
        fpath = sources_dir / entry["file"]
        if not fpath.exists():
            errors.append(f"missing file: {entry['file']}")
            continue
        source = json.loads(fpath.read_text(encoding="utf-8"))
        if source.get("sourceId") != entry["id"]:
            errors.append(
                f"sourceId mismatch in {entry['file']}: "
                f"{source.get('sourceId')} != {entry['id']}"
            )

    if errors:
        print(f"VALIDATION FAILED: {len(errors)} errors")
        for e in errors[:20]:
            print(f"  {e}")
        return 1

    print(f"validation passed: {manifest['total_sources']} sources OK")
    return 0


# ---------------------------------------------------------------------------
# 子命令: report
# ---------------------------------------------------------------------------

def cmd_report(args):
    """汇总评估看板."""
    evidence_path = Path(args.evidence) if args.evidence else None
    batch_path = Path(args.batch) if args.batch else None
    blockers_path = Path(args.blockers) if args.blockers else None
    out_path = Path(args.out)

    lines = ["# Reader 能力评估看板", ""]
    lines.append(
        f"生成时间: {datetime.now(timezone.utc).isoformat()}"
    )
    lines.append("")

    # 批量测试结果
    if batch_path and batch_path.exists():
        batch = json.loads(batch_path.read_text(encoding="utf-8"))
        lines.append("## 书源批量测试")
        lines.append("")
        s = batch.get("summary", {})
        lines.append(f"- 总数: {batch.get('total', '?')}")
        lines.append(f"- 完全通过: {s.get('fully_passed', '?')}")
        lines.append(f"- 部分通过: {s.get('partially_passed', '?')}")
        lines.append(f"- 完全失败: {s.get('fully_failed', '?')}")
        lines.append(f"- 通过率: {s.get('pass_rate', '?')}")
        lines.append("")
        by_level = s.get("by_level", {})
        if by_level:
            lines.append("### 按级别")
            lines.append("")
            lines.append("| 级别 | 通过 | 失败 |")
            lines.append("| --- | --- | --- |")
            for level in ["L1-import", "L2-search", "L3-detail", "L4-toc", "L5-content"]:
                lv = by_level.get(level, {})
                lines.append(
                    f"| {level} | {lv.get('passed', '?')} | {lv.get('failed', '?')} |"
                )
            lines.append("")

    # Blockers
    if blockers_path and blockers_path.exists():
        blockers = json.loads(blockers_path.read_text(encoding="utf-8"))
        lines.append("## Release Blockers")
        lines.append("")
        summary = blockers.get("summary", {})
        lines.append(f"- total: {summary.get('total', '?')}")
        lines.append(f"- blocker: {summary.get('blocker', '?')}")
        lines.append(f"- medium: {summary.get('medium', '?')}")
        lines.append(f"- low: {summary.get('low', '?')}")
        lines.append("")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"report → {out_path}")
    return 0


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="Legado 书源 corpus 管理工具"
    )
    sub = parser.add_subparsers(dest="command", required=True)

    p_import = sub.add_parser("import", help="导入书源集合")
    p_import.add_argument("--from", dest="from_file", required=True)
    p_import.add_argument("--to", required=True)
    p_import.add_argument("--no-sanitize", action="store_true")
    p_import.set_defaults(func=cmd_import)

    p_classify = sub.add_parser("classify", help="分类索引")
    p_classify.add_argument("--sources", required=True)
    p_classify.add_argument("--out", required=True)
    p_classify.set_defaults(func=cmd_classify)

    p_sanitize = sub.add_parser("sanitize", help="脱敏扫描")
    p_sanitize.add_argument("--sources", required=True)
    p_sanitize.add_argument("--report", required=True)
    p_sanitize.set_defaults(func=cmd_sanitize)

    p_validate = sub.add_parser("validate", help="验证完整性")
    p_validate.add_argument("--manifest", required=True)
    p_validate.set_defaults(func=cmd_validate)

    p_report = sub.add_parser("report", help="评估看板")
    p_report.add_argument("--evidence", default=None)
    p_report.add_argument("--batch", default=None)
    p_report.add_argument("--blockers", default=None)
    p_report.add_argument("--out", required=True)
    p_report.set_defaults(func=cmd_report)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
