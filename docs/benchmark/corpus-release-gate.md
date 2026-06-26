# Corpus benchmark & release gate

本文档记录 Reader-Core-Native 的 corpus benchmark / release-gate 工具链。
这条链路只处理已经产出的 JSON 结果文件，用 `canonicalizer`、`cross-platform-diff`、
`benchmark-run-packager` 和 `release-blocker-register` 建立可重复证据：同一
`input` 下，`cli`、`ios`、`android`、`harmony` 四个 candidate 是否能归一化为同一份
canonical result。
其中 `cli` 是 Rust/Core 参考候选；真正对应用户端迁移证明的是
`ios`、`android`、`harmony` 三端 `hostParity`。
collector 还会输出 `corpusProof`，把 source manifest、同一 Rust Core 身份、四端 diff、
三端 `hostParity` 和 open blocker 汇总成 `pass` / `blocked` 的 corpus 证据状态。
`corpusProof.status=pass` 要求 manifest 显式声明 `schemaVersion=1`，并匹配本次
`runId/scenario`、原始 artifact `sha256`、整体 `expected.match/total`、完整
`expected.byPlatform` 和 `expected.hostParity`；只靠实际 diff 碰巧为 0 不会通过。

重要边界：这里的通过结果只是 corpus/diff 工具证据，不代表 iOS / Android / Harmony
真实 App adapter 已完成，也不代表平台 host 已经接入 Core。

## 范围与约束

本路线只触碰这些路径：

| Piece | Location |
|-------|----------|
| Canonicalizer | `scripts/corpus_canonicalize.py` |
| Real-run collector | `tools/corpus-real-run-collector/` |
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
- 不把 CLI 结果当成三端 proof。三端同结果必须看 collector / packager 输出中的
  `hostParity`，它只统计 `ios`、`android`、`harmony`。
- 不把 `corpusProof.status=pass` 当成 App/device release proof。它只说明本地 JSON corpus
  证据满足 source manifest、同 Core 身份、四端 diff、三端 host parity 和 blocker 条件。
- 不允许缺少预期声明的 run 变成最终证明。`expected.byPlatform` 和 `expected.hostParity`
  缺失时，collector 仍可归档结果，但 `corpusProof.status` 必须是 `blocked`。
- 不允许拿 A manifest 包装成 B run。manifest 如果声明 `runId` 或 `scenario`，必须与
  collector 参数一致；缺少这两个绑定时，`corpusProof.status` 也不能是 `pass`。
- 不允许无 schema 的 manifest 成为最终证明。缺失 `schemaVersion` 时可以归档但
  `corpusProof.status=blocked`；不支持的 `schemaVersion` 会被 collector 拒绝。
- 不允许只靠路径绑定内容。`corpusProof.status=pass` 要求 source manifest 声明
  `artifacts.input`、`artifacts.canonical` 和四端 `artifacts.candidates.*` 的 raw
  file `sha256`，且必须与本次传入文件一致。
- source manifest 本身也必须进入输出证据包。传入 `--source-manifest` 时，collector
  会复制原始 manifest 到 `raw/source-manifest.json`，并在输出 `manifest.json` 的
  `sourceManifestFile` 中记录 source/package `sha256`。
- 生成物默认写入 `/private/tmp`，不要在 `~/Documents` 或仓库工作区里写 release 产物。

## 四端 fixture

新增 fixture 位于 `samples/corpus-release-gate/`：

| Fixture | Purpose | Expected diff |
|---------|---------|---------------|
| `four-platform-match/` | 同一 `input` 下，`cli`、`ios`、`android`、`harmony` 四个 candidate 只存在字段顺序、空白、HTML entity、URL trailing slash、timestamp / traceId 等可归一化差异 | `match=true`, `total=0` |
| `four-platform-mismatch/` | 同一 `input` 下，只有 `android` 的 `results[1].name` 从 `River Notes` 变为 `River Note` | `match=false`, `total=1`, blocker platform=`android` |

每个 fixture 都包含：

- `input.json`：同一输入声明。
- `manifest.json`：candidate 路径、同一 Rust Core 身份和预期 diff 摘要。
- `canonical-result.json`：canonical reference。
- `candidates/cli-result.json`
- `candidates/ios-result.json`
- `candidates/android-result.json`
- `candidates/harmony-result.json`

## Pipeline

```text
real-run candidate JSON
(cli / ios / android / harmony)
        |
        v
tools/corpus-real-run-collector/corpus_real_run_collector.py
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

### 1. Real-run collector

`tools/corpus-real-run-collector/corpus_real_run_collector.py` 把已经由
CLI / iOS / Android / HarmonyOS 真实运行产出的本地 JSON 文件收成统一 candidate package。
它不运行 Core、不运行平台 adapter、不联网，只复制本地 JSON、生成 canonicalized candidate、
执行四端 diff，并把 diff 产生的差异写入 blocker register。

collector 输出目录同时满足 `benchmark-run-packager` 的 run directory 结构：

```text
candidate-package/
  manifest.json
  platform-result.json
  canonical-result.json
  diff-result.json
  environment.json
  corpus-blocker-register.json
  input.json
  raw/
    source-manifest.json
    canonical-result.json
    cli-result.json
    ios-result.json
    android-result.json
    harmony-result.json
  candidates/
    cli-result.json
    ios-result.json
    android-result.json
    harmony-result.json
```

传入 `--source-manifest` 时，collector 会把 manifest 中的 `input`、`canonical` 和
`candidates.{cli,ios,android,harmony}` 解析到 manifest 所在目录，并要求它们与本次命令
实际传入的文件路径一致。manifest 还必须声明 `coreIdentity` 与
`platformRuns.{cli,ios,android,harmony}`，并满足：

- `schemaVersion` 必须是 `1`；不支持的 schema 会被拒绝；
- 如果 manifest 提供 `runId` 或 `scenario`，它们必须与本次 collector 参数完全一致；
- `corpusProof.status=pass` 要求 manifest 同时声明 `runId` 和 `scenario`；
- `artifacts.input`、`artifacts.canonical` 和
  `artifacts.candidates.{cli,ios,android,harmony}` 必须是对应 raw 文件的 SHA-256；
- `businessKernel` 必须是 `reader-core-native-rust`；
- 四端 `coreCommit`、`abiVersion`、`protocolVersion` 必须与 `coreIdentity` 完全一致；
- 如果 manifest 提供 `expected.match` 或 `expected.total`，实际 diff 结果也必须一致；
- 如果 manifest 提供 `expected.byPlatform`，`cli`、`ios`、`android`、`harmony`
  每端的 `match` / `total` 都必须与实际 diff summary 一致；
- 如果 manifest 提供 `expected.hostParity`，`ios`、`android`、`harmony` 三端的
  聚合 `match` / `total` 必须与实际 host parity 一致；
- 如果 manifest 提供 `expected.blockerPlatform` / `expected.blockerPath`，实际 diff
  必须在指定平台和字段路径产生 blocker 差异。

这让 source manifest 成为同一 Rust Core 身份、输入来源和预期结果约束，而不只是被动
复制到输出包。collector 输出的 `sourceManifestFile` 记录原始 source manifest 的
`packagePath`、`sourceSha256` 和 `packageSha256`，用于在后续 bundle 审查中核对证明声明
本身没有被路径替换或丢失。

示例：

```bash
python3 tools/corpus-real-run-collector/corpus_real_run_collector.py \
  --run-id fixture-four-platform-match \
  --scenario four-platform-search \
  --input samples/corpus-release-gate/four-platform-match/input.json \
  --source-manifest samples/corpus-release-gate/four-platform-match/manifest.json \
  --canonical samples/corpus-release-gate/four-platform-match/canonical-result.json \
  --candidate cli:samples/corpus-release-gate/four-platform-match/candidates/cli-result.json \
  --candidate ios:samples/corpus-release-gate/four-platform-match/candidates/ios-result.json \
  --candidate android:samples/corpus-release-gate/four-platform-match/candidates/android-result.json \
  --candidate harmony:samples/corpus-release-gate/four-platform-match/candidates/harmony-result.json
```

默认输出在 `/private/tmp/<run-id>-candidate`。如果四端 diff 存在差异，
collector 会把对应差异写入该目录下的 `corpus-blocker-register.json`；
是否关闭或 waiver 这些 blocker 由后续审查决定。

### 2. Canonicalizer

`scripts/corpus_canonicalize.py` 将 JSON result 归一化为稳定可比较的 canonical JSON：

- object keys 递归排序；
- 非换行空白折叠，行首行尾空白清理，首尾空行清理；
- CRLF / CR 归一化为 LF；
- HTML named / numeric entities 解码；
- URL path 末尾单个 `/` 归一化；
- 顶层 `timestamp`、`request_id`、`traceId` 等 run-variable metadata 写为 `<normalized>`；
- `updated_at`、`date`、`time` 以及嵌套 `results[].timestamp` 这类业务字段保留并参与 diff。

```bash
python3 scripts/corpus_canonicalize.py input.json -o output.json
```

### 3. Cross-platform diff

`tools/cross-platform-diff/cross_platform_diff.py` 对 canonical reference 和多个 named
candidate 先做 canonicalize，再生成 `diff-result.json`。
`diff-result.json` 同时记录：

- raw input `sha256`；
- 用当前 canonicalizer 策略生成的 `canonicalizedSha256`；
- `normalization` 策略摘要，包括顶层 run-variable metadata 字段。

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

### 4. Run packager

`tools/benchmark-run-packager/benchmark_run_packager.py` 只打包已经存在的 run directory：

```text
run-dir/
  manifest.json
  platform-result.json
  canonical-result.json
  diff-result.json
```

它会生成 `summary.json`，但不会运行 benchmark，也不会调用 Core。
普通 run directory 只要求上述四个 JSON artifact；如果 manifest 是 real-run collector
输出，packager 会额外验证 manifest 声明的包内 artifact：

- `sourceManifestFile.packagePath` 必须存在于 run directory 内，且
  `sourceSha256` / `packageSha256` 必须匹配；
- `input.packagePath`、`canonical.rawPath`、`canonical.packagePath` 必须存在，且各自
  package/raw SHA-256 必须匹配；
- `candidates.*.rawPath` 和 `candidates.*.canonicalizedPath` 必须存在，且
  `rawSha256` / `sourceSha256` / `canonicalizedFileSha256` 必须匹配；
- `artifacts.*` 中声明的路径必须留在 run directory 内并存在。

packager 还会用包内 `diff-result.json` 和 `corpus-blocker-register.json` 重新校验
collector manifest 摘要：

- `manifest.diffSummary` 必须等于 `diff-result.json` 的 `match` / `total` /
  `summary`；
- `manifest.hostParity` 必须能从 `diff-result.json.summary` 的
  `ios`、`android`、`harmony` 三端结果重新推出；
- `manifest.blockers.open` 和 `manifest.blockers.openByPlatform` 必须等于包内
  blocker register 对当前 `runId` 的 open blocker 状态；
- `corpusProof.conditions.fullDiffMatch`、`hostParityMatch`、`openBlockers` 必须与
  包内 diff/register 一致；如果 `corpusProof.status=pass`，包内 diff 必须 match、
  三端 host parity 必须 match，且 open blocker 必须为 0。

这些校验失败时，packager 会拒绝生成 bundle；不能把缺文件、hash 已变或摘要与包内
JSON 不一致的 collector run 归档成最终 corpus 证据。

packager 生成 bundle 后还会写入并立即验证归档自描述文件：

- `bundle-manifest.json`：列出 bundle payload 文件，包括 `summary.json`、run 原始
  artifact、collector raw/canonicalized artifact，以及每个文件的 size / SHA-256；
- `bundle-manifest.sha256`：记录 `bundle-manifest.json` 本身的 SHA-256；
- `verify_bundle_manifest()` 会拒绝 payload 文件被修改、删除、额外加入或 manifest
  checksum 不匹配的 bundle。
- `--verify-bundle` 还会从包内 payload 重新计算 `validation`、`diffSummary`
  和 `evidence`，并确认它们与 `summary.json` 中的记录一致；这会覆盖
  `manifest.json`、`diff-result.json`、`corpus-blocker-register.json` 与 collector
  raw/canonicalized artifact 的一致性。只重写 `summary.json`、`bundle-manifest.json`
  和 checksum 的自洽 bundle 不能作为 corpus 证据通过。
- 默认 `--verify-bundle` 允许 `corpusProof.status=blocked` 的 negative evidence
  bundle 通过完整性校验，便于归档复核；最终三端同结果 proof 必须额外传
  `--require-corpus-proof-pass`，要求 `evidence.corpusProof.status=pass`、
  `evidence.hostParity.match=true`、`reasons=[]`、`missingCandidates=[]`、
  `missingSummary=[]`，并要求 `corpusProof.conditions` 中 source manifest、schema、
  core identity、artifact hashes、run/scenario binding、expected declarations、四端
  candidate/summary presence、`fullDiffMatch`、`hostParityMatch` 全部为 `true`，
  且 `openBlockers=0`。
- 最终 proof 还应传 `--require-core-commit <commit>`。该校验要求
  `evidence.coreIdentity.businessKernel=reader-core-native-rust`，并要求
  `evidence.coreIdentity.coreCommit` 与 `evidence.platforms.{cli,ios,android,harmony}.coreCommit`
  全部匹配同一个 Rust Core commit（支持 7-40 位小写 hex，短/长 hash 前缀匹配）。
- 最终 proof 还应传 `--require-run-id <run-id>` 和 `--require-scenario <scenario>`。
  这样一个通过的 bundle 只能证明指定 corpus run / scenario，不能被挪用到其他语义场景。

`bundle-manifest.json` 和 `bundle-manifest.sha256` 会进入 zip 输出；这让 zip 解包后也能
先验 manifest 本身，再验所有 payload 文件。CI 或人工审查可以直接运行：

```bash
python3 tools/benchmark-run-packager/benchmark_run_packager.py --verify-bundle /private/tmp/run-bundle
python3 tools/benchmark-run-packager/benchmark_run_packager.py --verify-bundle /private/tmp/run-bundle.zip
python3 tools/benchmark-run-packager/benchmark_run_packager.py \
  --verify-bundle /private/tmp/run-bundle \
  --require-corpus-proof-pass \
  --require-run-id "2026-06-25-real-001" \
  --require-scenario "authorized-corpus-search" \
  --require-core-commit "$(git rev-parse --short HEAD)"
```

`--verify-bundle` 对 bundle 目录和 zip 文件都可用；zip 校验直接读取压缩包中的
`bundle-manifest.json`、`bundle-manifest.sha256` 和 payload 条目，并在 `/private/tmp`
中创建一次性副本来重跑 payload validation，不会解包或写入工作区。

如果 run directory 来自 real-run collector，`summary.json` 会额外包含 `evidence`：

- `evidence.coreIdentity`：`businessKernel`、`coreCommit`、`abiVersion`、`protocolVersion`；
- `evidence.sourceManifestFile`：包内 `raw/source-manifest.json` 路径与 source/package
  `sha256`；
- `evidence.hostParity`：只面向 `ios`、`android`、`harmony` 的三端同结果摘要；
- `evidence.corpusProof`：`pass` / `blocked` 的 corpus 证据 gate，不等于 release ready；
- `evidence.platforms.{cli,ios,android,harmony}`：每端声明的 Core 身份、raw result 路径、
  canonicalized result 路径、参与 diff 比较的 `canonicalizedSha256`，以及包内
  canonicalized JSON 文件的 `canonicalizedFileSha256`。

这只是归档摘要，证明包内 artifact 与同一 Rust Core 身份绑定；仍不等于 App/device
发布完成。

```bash
python3 tools/benchmark-run-packager/benchmark_run_packager.py /private/tmp/run-dir --out /private/tmp/run-bundle
```

### 5. Release blocker register

`tools/release-blocker-register/release_blocker_register.py` 从 `diff-result.json` 中把非匹配
candidate 的差异登记为 blocker。它只维护 blocker 状态，不认证 release。
如果 `diff-result.json` 缺少 `cli`、`ios`、`android`、`harmony` 中任一 candidate，
也会登记 `missing-platform-candidate` blocker；单端或部分端 diff 不能让 gate 变绿。
由 enhanced `diff-result.json` 派生的 blocker 会同时保留 raw `sha256` 和
canonicalizer 策略后的 `canonicalizedSha256`，用于证明 blocker 指向的是实际参与比较的
规范化 JSON 内容。

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
2. package match run directory，生成 match bundle zip，并用 `--verify-bundle
   --require-corpus-proof-pass --require-run-id <run-id> --require-scenario <scenario>
   --require-core-commit <fixture-core-commit>` 验证 bundle 目录和 zip 可作为指定 run /
   scenario / Rust Core commit 的最终同结果 proof。
3. 跑 `four-platform-mismatch/`，断言 `total=1`。
4. package mismatch blocked run directory，并用 `--verify-bundle --require-run-id
   --require-scenario --require-core-commit` 验证 blocked evidence bundle 仍绑定指定 run /
   scenario / Rust Core commit 且可归档复核，同时断言它无法通过
   `--require-corpus-proof-pass`。
5. 从 mismatch diff 注册 blocker。
6. 断言 gate blocked。
7. 仅为 demo 收尾 close 该 blocker，再断言 gate clear。

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
- `tests/tooling/test_corpus_real_run_collector.py` 验证 collector 生成 packager-compatible
  candidate package，并把 mismatch 写入 blocker register。
