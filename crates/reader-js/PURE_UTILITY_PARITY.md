# Reader JS Pure Utility Parity Ledger

Last verified: 2026-06-25

Scope: Reader-Core-Native compatibility work limited to `crates/reader-js`,
`crates/reader-rule`, and `crates/reader-content`. Do not count ABI, protocol,
bindings, storage, or host-app runtime work as completed by this ledger.

Authoritative local evidence:

- Legado helper surface:
  `/Users/minliny/Documents/legado/app/src/main/java/io/legado/app/help/JsExtensions.kt`
- Legado URL object:
  `/Users/minliny/Documents/legado/app/src/main/java/io/legado/app/utils/JsURL.kt`
- Old Reader-Core safe utility allowlist:
  `/Users/minliny/Documents/Reader-Core/Sources/ReaderCoreAPI/JSGated/JSSandboxDynamicEvalRuntime.swift`
- Old Reader-Core fixture:
  `/Users/minliny/Documents/Reader-Core/samples/booksources/runtime_js_fixtures/pure_utility_bindings_expected.json`

Latest scoped verification:

```sh
cargo test -p reader-rule -p reader-js -p reader-content --quiet
```

Observed result at save time:

- `reader-content`: 36 passed
- `reader-js`: 93 passed
- `reader-rule`: 8 passed, 7 passed, 130 passed

## Saved Audit Snapshot

This file is the saved working inventory for the `reader-rule` /
`reader-js` / `reader-content` compatibility goal. It is intentionally narrower
than the complete Legado JS host surface: it tracks only helpers that can be
closed inside these crates without changing ABI, protocol, bindings, storage,
or host-app runtime behavior.

Old Reader-Core classified the following names as safe utility evaluation:

- `base64Decode`, `base64DecodeToByteArray`, `base64Encode`
- `md5Encode`, `md5Encode16`, `hashDigest`
- `hexDecode`, `hexDecodeToByteArray`, `hexDecodeToString`, `hexEncode`,
  `hexEncodeToString`
- `t2s`, `s2t`, `toNumChapter`
- `timeFormat`, `timeFormatUTC`
- `strToBytes`, `bytesToStr`
- `log`, `logType`, `toast`, `longToast`, `refreshTocUrl`
- `randomUUID`, `getWebViewUA`, `encodeURI`, `toURL`, `htmlFormat`
- `put`, `get`

For this native goal, `put` / `get` are not counted as pure helper closures
because they cross into context/state storage semantics. They should be handled
only if a future scoped round proves the behavior belongs in `reader-js`
without changing storage or host contracts.

## Current Completion Snapshot

| Status | Count | Capability groups |
| --- | ---: | --- |
| Closed for fixture or Core-intent path | 19 | Base64, byte/string round trip, hex, MD5, SHA-256 digest, HMAC-SHA256 digest, URI encoding, local time, UTC time, chapter-number normalization, HTML text formatting, URL resolution, debug log, toast intent, refresh TOC intent, JS type names, UUID, controlled WebView UA, host callback routing |
| Partially closed | 5 | Charset overloads, Base64 charset breadth, full `htmlFormat` image-preserving behavior, full Java URL edge cases, full traditional/simplified Chinese dictionary parity |
| Missing pure utility closure | 0 | No standalone pure utility closure remains listed in this ledger |
| Explicitly excluded from this ledger | uncounted | Real network, WebView/browser/captcha, file/archive/font/platform identity, ABI/protocol/bindings/storage/host app behavior |

## Capability Inventory

| Source capability | Native status | Current scope decision |
| --- | --- | --- |
| `base64Encode` / `base64Decode` / `base64DecodeToByteArray` | Closed for old Core fixture paths | Keep as deterministic JS helper coverage; expand only with new charset fixtures. |
| `strToBytes` / `bytesToStr` | Partially closed | UTF-8/default paths, byte-array decode paths, global `strToBytes`, and ISO-8859-1/Latin1 overload round trip are covered; broad charset overload parity remains open. |
| `hexEncode` / `hexDecode` / `hexEncodeToString` / `hexDecodeToString` / `hexDecodeToByteArray` | Closed for old Core fixture paths | Current aliases match the safe-utility fixture expectations. |
| `md5Encode` / `md5Encode16` | Closed for fixture paths | Broader MD5 behavior should stay deterministic and side-effect free. |
| `hashDigest` / `java.digestHex` / `java.digestBase64Str` | Closed for tested fixture paths | Covers `hashDigest` SHA-256 plus `java.digestHex` and `java.digestBase64Str` MD5/SHA-256 fixture paths. SHA-1, SHA-512, SM3, and global digest aliases are not claimed. |
| `hmacDigest` / `HMacHex` / `hmacHex` / `HMacBase64` / `hmacBase64` | Closed for HMAC-SHA256 fixture path | Covers Legado/old Core hex and Base64 HMAC aliases for the tested SHA-256 path; broader HMAC algorithm parity is not claimed. |
| `encodeURI` / `encodeURIComponent` | Closed for tested paths | Covers `java.*` and global aliases for UTF-8/default plus ISO-8859-1/Latin1 overload fixture paths; full charset-table breadth is not claimed. |
| `timeFormat` / `timeFormatUTC` | Closed for tested paths | Current tests pin deterministic UTC+8/default and UTC offset behavior. |
| `toNumChapter` | Closed for tested paths | Full regex/numeral parity can be expanded with additional fixtures. |
| `t2s` / `s2t` | Partially closed | Deterministic common subset only; full Legado/OpenCC-style table parity remains open. |
| `htmlFormat` | Partially closed | Text fixture is closed; Legado `formatKeepImg` image-preserving details remain open. |
| `toURL` | Partially closed | Old Core relative fixture is closed; complete Java URL edge-case parity remains open. |
| `log` / `logType` | Closed for Core observable path | Uses reader-js console capture; no host log side effect is claimed. |
| `toast` / `longToast` | Closed for Core intent-marker path | Uses reader-js console capture; no host UI side effect is claimed. |
| `refreshTocUrl` | Closed for Core intent-marker path | Returns empty string and records `java.refreshTocUrl() requested`; no host refresh behavior is claimed. |
| `randomUUID` | Closed for UUID v4 shape | Does not claim platform crypto entropy properties. |
| `getWebViewUA` | Closed for deterministic old Core default | Does not read host/platform WebView state. |
| `put` / `get` | Excluded from this pure-helper ledger | State/storage semantics are outside this goal unless separately scoped. |

## Closed In Current Worktree

| Capability | Entrypoints covered | Evidence status | Notes |
| --- | --- | --- | --- |
| Base64 encode/decode | `base64Encode`, `base64Decode`, `base64DecodeToByteArray`, and `java.*` variants | Closed for fixture paths | Covers URL-safe/no-padding flags, byte arrays, UTF-8, ISO-8859-1 fixture, and GBK fixture sample. |
| Byte array/string round trip | `strToBytes`, `bytesToStr`, and `java.*` variants | Closed for fixture paths | `bytesToStr(base64DecodeToByteArray("YWJj"))` returns `abc`; `java.bytesToStr` accepts JS byte arrays and prior hex-string path; ISO-8859-1/Latin1 overload round trip is covered. |
| Hex helpers | `hexEncode`, `hexDecode`, `hexEncodeToString`, `hexDecodeToString`, `hexDecodeToByteArray`, and `java.*` variants | Closed for fixture paths | Covers old Core global alias `hexDecodeToString(hexEncodeToString("abc"))`, UTF-8 round trip, and byte-array decode. |
| MD5 helpers | `md5Encode`, `md5Encode16`, and `java.*` variants | Closed for fixture paths | `md5Encode16` follows Legado `md5Encode(str).substring(8, 24)` and old Core fixture for `abc`. |
| Hash digest | `hashDigest`, `java.hashDigest`, `java.digestHex`, and `java.digestBase64Str` | Closed for tested fixture paths | Covers old Core `hashDigest("abc", "SHA-256")` fixture plus Legado `java.digestHex` and `java.digestBase64Str` MD5/SHA-256 fixture paths. SHA-1, SHA-512, SM3, global digest aliases, and broader `MessageDigest` algorithm parity are not claimed. |
| HMAC digest | `hmacDigest`, `HMacHex`, `hmacHex`, `HMacBase64`, `hmacBase64`, and `java.*` variants | Closed for HMAC-SHA256 fixture path | Covers old Core `hmacDigest(text, "HMAC-SHA256", "key")` fixture plus Legado/old Core hex and Base64 aliases. Broader HMAC algorithm parity is not claimed. |
| URI encoding | `encodeURI`, `encodeURIComponent`, and `java.*` variants | Closed for tested paths | Covers UTF-8/default paths plus ISO-8859-1/Latin1 charset overload fixtures for global aliases and `java.*`; full charset-table breadth is not claimed. |
| Local time formatting | `timeFormat` and `java.timeFormat` | Closed for tested paths | Covers old Core `timeFormat(ms, format)`/`java.timeFormat(ms, format)` with deterministic UTC+8 output and Legado `java.timeFormat(ms)` default `yyyy/MM/dd HH:mm` pattern. |
| Time UTC formatting | `timeFormatUTC`, `java.timeFormatUTC` | Closed for fixture path | Covers `yyyy`, `MM`, `dd`, `HH`, `mm`, `ss` token set and offset millis. |
| Chapter-number normalization | `toNumChapter` and `java.toNumChapter` | Closed for tested paths | Covers Legado `(第)(.+?)(章)` extraction, full-width digits, Chinese numerals, and pass-through when no title-number pattern matches. |
| HTML text formatting | `htmlFormat`, `java.htmlFormat` | Closed for fixture path | Covers tag stripping, block line breaks, and common HTML entities for the old Core fixture. |
| URL resolution | `toURL`, `java.toURL` | Closed for fixture path | Covers `String(toURL(...))`, `host`, `origin`, `pathname`, and basic `searchParams`. |
| Debug log helper | `log` and `java.log` | Closed for Core log-capture path | Returns the input message and records it through reader-js console capture. No stdout or host UI side effect is claimed. |
| Toast intent helpers | `toast`, `longToast`, and `java.*` variants | Closed for Core intent-marker path | Returns empty string and records marker/message/duration through reader-js console capture. No host UI side effect is claimed. |
| Refresh TOC intent helper | `refreshTocUrl` and `java.refreshTocUrl` | Closed for Core intent-marker path | Returns empty string and records `java.refreshTocUrl() requested` through reader-js console capture. No host refresh side effect is claimed. |
| JS type names | `logType`, `java.logType` | Closed for fixture path | Covers old Core object -> `object` and array -> `array`, plus stable null/string primitive names. No logging side effect is claimed. |
| Random UUID | `randomUUID` and `java.randomUUID` | Closed for UUID v4 shape | Covers Legado lowercase UUID string shape and successive-call difference. Does not claim host crypto entropy strength. |
| Controlled WebView UA | `getWebViewUA`, `java.getWebViewUA` | Closed for fixture path | Returns the old Core deterministic default UA. Does not read host/platform WebView state. |
| Host callback routing | `ajax`, `ajaxAll`, `getSource`, and `java.*` callback routes | Boundary closed | Proves routing to host callback registry only; does not implement host network/WebView/storage behavior. |

## Partially Closed

| Capability | Current coverage | Remaining gap |
| --- | --- | --- |
| String/byte charset overloads | UTF-8 default paths, ISO-8859-1/Latin1 round trip, `bytesToStr` arrays/hex strings, and global `strToBytes` alias are covered | Full charset-table parity is not broadly covered; GBK/GB18030 encode/decode remains fixture-subset level. |
| Base64 charset | ISO-8859-1 fixture and GBK fixture sample covered | Full GBK/GB18030 table is not implemented; current GBK path is fixture-sample level. |
| HTML formatting | Old Core text fixture covered | Full Legado `HtmlFormatter.formatKeepImg` behavior, especially image-preserving details, is not fully reproduced. |
| URL parsing | Old Core relative-path fixture covered | Full Java `URL(base, relative)` equivalence is not proven for every edge case. |
| Traditional/simplified Chinese conversion | `t2s`, `s2t`, and `java.*` variants cover a deterministic common-character subset shared by old Core and Legado | Full Legado quick-transfer/OpenCC-style dictionary parity is not implemented. |

## Missing Pure Utility Closures

No standalone pure utility closure remains listed here at this save point.
Future rounds should take the next smallest item from `Partially Closed` and
close it with local old-behavior evidence, a minimal failing test,
implementation, and scoped test evidence.

## Excluded Or Host-Bound For This Goal

Do not mark these complete from `reader-js` pure helpers alone:

- Real network execution: `get`, `post`, `connect`, `ajax`
- WebView/browser/captcha: `webView`, `startBrowser`, `getVerificationCode`
- File/archive/font/platform identity APIs
- Toast/UI side effects unless explicitly implemented as deterministic no-op
- ABI, protocol, bindings, storage, host app behavior

## Operating Rule For Future Rounds

Each next round should choose one item from `Partially Closed` unless a newly
discovered pure utility closure is added back to `Missing Pure Utility
Closures`, then:

1. Reconfirm current code and local Legado/old Reader-Core behavior.
2. Add the smallest failing test.
3. Implement only inside `crates/reader-js`, `crates/reader-rule`, or
   `crates/reader-content`.
4. Run focused test, `cargo test -p reader-js --quiet`, scoped three-crate
   tests, formatting, and `git diff --check`.
5. Update this ledger if the status changes.
