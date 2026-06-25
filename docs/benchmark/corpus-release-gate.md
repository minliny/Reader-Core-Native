# Corpus benchmark & release gate

本文档记录 Reader-Core-Native 的 corpus benchmark / release-gate 工具链。
这条链路只处理已经产出的 JSON 结果文件，用 `canonicalizer`、`cross-platform-diff`、
`benchmark-run-packager` 和 `release-blocker-register` 建立可重复证据：同一
`input` 下，`cli`、`ios`、`android`、`harmony` 四个 candidate 是否能归一化为同一份
canonical result。

重要边界：这里的通过结果只是 corpus/diff 工具证据，不代表 iOS / Android / Harmony
真实 App adapter 已完成，也不代表平台 host 已经接入 Core。

## 范围与约束

本路线只触碰这些路径：

| Piece | Location |
|-------|----------|
| Canonicalizer | `scripts/corpus_canonicalize.py` |
| Demo script | `scripts/corpus_release_gate_demo.sh` |
| Cross-platform diff | `tools/cross-platform-diff/` |
| Run packager | `tools/benchmark-run-packager/` |
| Release blocker register | `tools/release-blocker-register/` |
| Sample corpus | `samples/corpus-release-gate/` |
| Tests | `tests/tooling/` |
| Docs | `docs/benchmark/` |

硬约束：

- 不改 `crates/**`、`protocol/**`、`bindings/**`、`tools/reader-cli/**`。
- 不运行 Core business logic，不实现平台 adapter，不把 host app 能力伪装成 Core 完成。
- 不声明 `release ready`。`release-blocker-register gate` 只报告 open blocker 数量和退出码。
- 不把单端结果伪装成四端一致。四端一致至少需要同一个 `diff-result.json` 同时携带
  `cli`、`ios`、`android`、`harmony` 四个 candidate。
- 生成物默认写入 `/private/tmp`，不要在 `~/Documents` 或仓库工作区里写 release 产物。

## 四端 fixture

新增 fixture 位于 `samples/corpus-release-gate/`：

| Fixture | Purpose | Expected diff |
|---------|---------|---------------|
| `four-platform-match/` | 同一 `input` 下，`cli`、`ios`、`android`、`harmony` 四个 candidate 只存在字段顺序、空白、HTML entity、URL trailing slash、timestamp / traceId 等可归一化差异 | `match=true`, `total=0` |
| `four-platform-mismatch/` | 同一 `input` 下，只有 `android` 的 `results[1].name` 从 `River Notes` 变为 `River Note` | `match=false`, `total=1`, blocker platform=`android` |

每个 fixture 都包含：

- `input.json`：同一输入声明。
- `manifest.json`：candidate 路径和预期 diff 摘要。
- `canonical-result.json`：canonical reference。
- `candidates/cli-result.json`
- `candidates/ios-result.json`
- `candidates/android-result.json`
- `candidates/harmony-result.json`

## Pipeline

```text
candidate JSON
(cli / ios / android / harmony)
        |
        v
scripts/corpus_canonicalize.py
        |
        v
tools/cross-platform-diff/cross_platform_diff.py
        |
        +--> diff-result.json
        |
        +--> tools/benchmark-run-packager/benchmark_run_packager.py
        |
        +--> tools/release-blocker-register/release_blocker_register.py
```

### 1. Canonicalizer

`scripts/corpus_canonicalize.py` 将 JSON result 归一化为稳定可比较的 canonical JSON：

- object keys 递归排序；
- 非换行空白折叠，行首行尾空白清理，首尾空行清理；
- CRLF / CR 归一化为 LF；
- HTML named / numeric entities 解码；
- URL path 末尾单个 `/` 归一化；
- `timestamp`、`request_id`、`traceId` 等 run-variable fields 写为 `<normalized>`。

```bash
python3 scripts/corpus_canonicalize.py input.json -o output.json
```

### 2. Cross-platform diff

`tools/cross-platform-diff/cross_platform_diff.py` 对 canonical reference 和多个 named
candidate 先做 canonicalize，再生成 `diff-result.json`。

四端 match fixture 示例：

```bash
python3 tools/cross-platform-diff/cross_platform_diff.py \
  samples/corpus-release-gate/four-platform-match/canonical-result.json \
  --candidate cli:samples/corpus-release-gate/four-platform-match/candidates/cli-result.json \
  --candidate ios:samples/corpus-release-gate/four-platform-match/candidates/ios-result.json \
  --candidate android:samples/corpus-release-gate/four-platform-match/candidates/android-result.json \
  --candidate harmony:samples/corpus-release-gate/four-platform-match/candidates/harmony-result.json \
  -o /private/tmp/four-platform-match-diff-result.json
```

预期：

```json
{
  "match": true,
  "total": 0
}
```

四端 mismatch fixture 示例：

```bash
python3 tools/cross-platform-diff/cross_platform_diff.py \
  samples/corpus-release-gate/four-platform-mismatch/canonical-result.json \
  --candidate cli:samples/corpus-release-gate/four-platform-mismatch/candidates/cli-result.json \
  --candidate ios:samples/corpus-release-gate/four-platform-mismatch/candidates/ios-result.json \
  --candidate android:samples/corpus-release-gate/four-platform-mismatch/candidates/android-result.json \
  --candidate harmony:samples/corpus-release-gate/four-platform-mismatch/candidates/harmony-result.json \
  -o /private/tmp/four-platform-mismatch-diff-result.json
```

预期：`android` 产生 1 个 `value-mismatch`，路径为 `results[1].name`。

### 3. Run packager

`tools/benchmark-run-packager/benchmark_run_packager.py` 只打包已经存在的 run directory：

```text
run-dir/
  manifest.json
  platform-result.json
  canonical-result.json
  diff-result.json
```

它会生成 `summary.json`，但不会运行 benchmark，也不会调用 Core。

```bash
python3 tools/benchmark-run-packager/benchmark_run_packager.py /private/tmp/run-dir --out /private/tmp/run-bundle
```

### 4. Release blocker register

`tools/release-blocker-register/release_blocker_register.py` 从 `diff-result.json` 中把非匹配
candidate 的差异登记为 blocker。它只维护 blocker 状态，不认证 release。

```bash
python3 tools/release-blocker-register/release_blocker_register.py \
  --register /private/tmp/corpus-blocker-register.json \
  add-from-diff /private/tmp/four-platform-mismatch-diff-result.json \
  --run-id fixture-four-platform-mismatch \
  --severity high

python3 tools/release-blocker-register/release_blocker_register.py \
  --register /private/tmp/corpus-blocker-register.json \
  gate --run-id fixture-four-platform-mismatch
```

对 `four-platform-mismatch/`，gate 应该返回非 0，因为存在 open blocker：
`android results[1].name`。

## End-to-end demo

`scripts/corpus_release_gate_demo.sh` 在 `/private/tmp` 下执行完整工具链：

1. 跑 `four-platform-match/`，断言四端 `total=0`。
2. package match run directory。
3. 跑 `four-platform-mismatch/`，断言 `total=1`。
4. 从 mismatch diff 注册 blocker。
5. 断言 gate blocked。
6. 仅为 demo 收尾 close 该 blocker，再断言 gate clear。

```bash
bash scripts/corpus_release_gate_demo.sh
```

## Tests

tooling 测试使用 stdlib `unittest`：

```bash
python3 -m unittest discover -s tests/tooling -q
```

本路线新增的关键断言：

- `tests/tooling/test_cross_platform_diff.py` 验证 `four-platform-match/` 的四个 candidate
  全部 `match=true`、`total=0`。
- `tests/tooling/test_cross_platform_diff.py` 验证 `four-platform-mismatch/` 只有
  `android results[1].name` 一个 mismatch。
- `tests/tooling/test_release_blocker_register.py` 验证 mismatch diff 会生成一个
  `android` blocker。
