# Reader JS Pure Utility Parity Ledger

Last verified: 2026-06-25

Scope for route 3: Reader-Core-Native JS runtime compatibility work limited to
`crates/reader-js`. Do not count ABI, protocol, bindings, storage, BookSource
model, `reader-rule`, or host-app runtime work as completed by this ledger.

Authoritative local evidence:

- Legado helper surface:
  `/Users/minliny/Documents/legado/app/src/main/java/io/legado/app/help/JsExtensions.kt`
- Legado URL object:
  `/Users/minliny/Documents/legado/app/src/main/java/io/legado/app/utils/JsURL.kt`
- Old Reader-Core safe utility allowlist:
  `/Users/minliny/Documents/Reader-Core/Sources/ReaderCoreAPI/JSGated/JSSandboxDynamicEvalRuntime.swift`
- Old Reader-Core fixture:
  `/Users/minliny/Documents/Reader-Core/samples/booksources/runtime_js_fixtures/pure_utility_bindings_expected.json`
- Old Reader-Core symmetric crypto fixture:
  `/Users/minliny/Documents/Reader-Core/samples/booksources/runtime_js_fixtures/symmetric_crypto_bindings_expected.json`

Latest scoped verification:

```sh
cargo test -p reader-js --quiet
```

Observed result at save time:

- `reader-js`: 94 unit tests passed
- `reader-js` integration tests: 32 passed

## Saved Audit Snapshot

This file is the saved working inventory for the route 3 `reader-js`
compatibility goal. It is intentionally narrower
than the complete Legado JS host surface: it tracks only helpers that can be
closed inside this crate without changing ABI, protocol, bindings, storage,
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
- `createSign`
- legacy global `escape` / `unescape`
- `put`, `get`

For this native goal, `put` / `get` are not counted as pure helper closures
because they cross into context/state storage semantics. They should be handled
only if a future scoped round proves the behavior belongs in `reader-js`
without changing storage or host contracts.

## Current Completion Snapshot

| Status | Count | Capability groups |
| --- | ---: | --- |
| Closed for fixture or Core-intent path | 22 | Base64, byte/string round trip and byte-array concat, hex, MD5/SM3/SHA-1/SHA-256/SHA-512 digest, HMAC-MD5/HMAC-SHA1/HMAC-SHA256/HMAC-SHA512 digest, canonical signing, AES/DES/3DES/SM4 symmetric crypto fixture paths, URI encoding, legacy escape/unescape, local time, UTC time, chapter-number normalization, HTML text formatting, URL resolution, debug log, toast intent, refresh TOC intent, JS type names, UUID, controlled WebView UA, host callback routing |
| Partially closed | 6 | Charset overloads, Base64 charset breadth, symmetric crypto breadth, full `htmlFormat` image-preserving behavior, full Java URL edge cases, full traditional/simplified Chinese dictionary parity |
| Missing pure utility closure | 0 | No standalone pure utility closure remains listed in this ledger |
| Explicitly excluded from this ledger | uncounted | Real network, WebView/browser/captcha, file/archive/font/platform identity, ABI/protocol/bindings/storage/host app behavior |

## Capability Inventory

| Source capability | Native status | Current scope decision |
| --- | --- | --- |
| `base64Encode` / `base64Decode` / `base64Decoder` / `base64DecodeToByteArray` | Closed for old Core fixture paths | Keep as deterministic JS helper coverage; `base64Decoder` is an alias for legacy guarded-corpus fragments. Expand only with new charset fixtures. |
| `strToBytes` / `bytesToStr` | Partially closed | UTF-8/default paths, byte-array decode paths, global `strToBytes`, ISO-8859-1/Latin1 overload round trip, and GBK fixture keywords are covered; broad charset overload parity remains open. |
| `hexEncode` / `hexDecode` / `hexEncodeToString` / `hexDecodeToString` / `hexDecodeToByteArray` | Closed for old Core fixture paths | Current aliases match the safe-utility fixture expectations. |
| `md5Encode` / `md5Encode16` | Closed for fixture paths | Broader MD5 behavior should stay deterministic and side-effect free. |
| `hashDigest` / `java.digestHex` / `java.digestBase64Str` | Closed for tested fixture paths | Covers `hashDigest` SM3/SHA-1/SHA-256/SHA-512 plus `java.digestHex` and `java.digestBase64Str` MD5/SM3/SHA-1/SHA-256/SHA-512 fixture paths, including old Core empty-string behavior for unsupported digest algorithms. Global digest aliases are not claimed. |
| `hmacDigest` / `HMacHex` / `hmacHex` / `HMacBase64` / `hmacBase64` | Closed for tested HMAC fixture paths | Covers Legado/old Core hex and Base64 HMAC aliases for tested MD5/SHA-1/SHA-256/SHA-512 paths; broader HMAC input encoding parity is not claimed. |
| `createSign` | Closed for old Core fixture path | Covers sorted truthy-parameter canonicalization with default/explicit HMAC-MD5; no cookie, network, source, or host state is read. |
| `createSymmetricCrypto` / `encryptBase64` / `encryptHex` / `decryptHex` / `aesBase64DecodeToString` / `aesEncodeToBase64String` / `java.desEncodeToBase64String` / `java.tripleDESEncodeBase64Str` | Closed for AES/DES/3DES/SM4 fixture paths; partially closed overall | Covers old Core AES/CBC PKCS5/PKCS7/ZeroPadding, DES/CBC/PKCS5, DESede/CBC/PKCS5, 3DES two-key, SM4/CBC/PKCS7, and SM4/ECB/PKCS5 Base64 encrypt/decrypt fixtures plus object `encryptBase64(...)`, `encryptHex(...)`, and `decryptHex(...)` fixture paths, Legado-style `java.aesBase64DecodeToString`, guarded-corpus `java.aesEncodeToBase64String`, old source `java.desEncodeToBase64String(...)` Yueyou DES/ECB `_p` fixture, old source `java.tripleDESEncodeBase64Str(...)` QDSign fixture, AES/CBC byte-array key/IV fixture, and AES/CBC byte-array `decrypt(...)` / `decryptStr(...)` fixture paths; global `tripleDESEncodeBase64Str`, DES/3DES decode or alternate aliases, and broader binary transformation behavior are not claimed. |
| `encodeURI` / `encodeURIComponent` | Closed for tested paths | Covers `java.*` and global aliases for UTF-8/default, ISO-8859-1/Latin1 overload fixture paths, and GBK fixture keywords; full charset-table breadth is not claimed. |
| `escape` / `unescape` | Closed by QuickJS builtin plus integration coverage | Covers old guarded-corpus `%uXXXX` and `escape(result)` patterns without native host behavior. |
| `timeFormat` / `timeFormatUTC` | Closed for tested paths | Current tests pin deterministic UTC+8/default and UTC offset behavior. |
| `toNumChapter` | Closed for tested paths | Full regex/numeral parity can be expanded with additional fixtures. |
| `t2s` / `s2t` | Partially closed | Deterministic common subset plus old Reader-Core postprocessor fixture path is covered; full Legado/OpenCC-style table parity remains open. |
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
| Base64 encode/decode | `base64Encode`, `base64Decode`, `base64Decoder`, `base64DecodeToByteArray`, and `java.*` variants | Closed for fixture paths | Covers URL-safe/no-padding flags, byte arrays, UTF-8, ISO-8859-1 fixture, GBK `小说`, GBK `鬼吹灯` / `搜索` / `提交` fixture keywords, and the legacy guarded-corpus `java.base64Decoder` alias. |
| Byte array/string round trip and concat | `strToBytes`, `bytesToStr`, `Buffer.concat`, and `java.*` variants | Closed for fixture paths | `bytesToStr(base64DecodeToByteArray("YWJj"))` returns `abc`; `java.bytesToStr` accepts JS byte arrays and prior hex-string path; `Buffer.concat([...])` returns a JS byte array for the old imageDecode concat boundary; ISO-8859-1/Latin1 round trip and GBK `鬼吹灯` / `搜索` / `提交` fixture keywords are covered. |
| Hex helpers | `hexEncode`, `hexDecode`, `hexEncodeToString`, `hexDecodeToString`, `hexDecodeToByteArray`, and `java.*` variants | Closed for fixture paths | Covers old Core global alias `hexDecodeToString(hexEncodeToString("abc"))`, UTF-8 round trip, and byte-array decode. |
| MD5 helpers | `md5Encode`, `md5Encode16`, and `java.*` variants | Closed for fixture paths | `md5Encode16` follows Legado `md5Encode(str).substring(8, 24)` and old Core fixture for `abc`. |
| Hash digest | `hashDigest`, `java.hashDigest`, `java.digestHex`, and `java.digestBase64Str` | Closed for tested fixture paths | Covers old Core `hashDigest("abc", "SHA-256")` fixture plus old Core `digestHex("abc", "SHA-1")` / `digestHex("abc", "SHA-512")` / SM3 standard and empty-string vectors, Legado `java.digestHex` / `java.digestBase64Str` MD5/SM3/SHA-1/SHA-256/SHA-512 fixture paths, and old Core empty-string behavior for unsupported digest algorithms. Global digest aliases and broader `MessageDigest` API parity are not claimed. |
| HMAC digest | `hmacDigest`, `HMacHex`, `hmacHex`, `HMacBase64`, `hmacBase64`, and `java.*` variants | Closed for tested fixture paths | Covers old Core `hmacDigest(text, "HMAC-SHA256", "key")` fixture plus Legado/old Core hex and Base64 aliases, old source Qidian `java.HMacBase64(..., "HMAC-SHA1", aid).slice(0, -4)` key fixture, HMAC-MD5 through the `createSign` fixture, and old Core `HMAC-SHA512` / `HMAC-SHA-512` algorithm aliases. Broader HMAC input encoding parity is not claimed. |
| Canonical signing | `createSign`, `java.createSign` | Closed for old Core fixture path | Covers sorted truthy params and default/explicit `HMAC-MD5` for `method=...&spaceId=...&timestamp=...`; this is pure signing only, not request execution. |
| AES/DES/3DES/SM4 symmetric crypto | `createSymmetricCrypto`, `java.createSymmetricCrypto`, object `encryptBase64`, object `encryptHex`, object `decryptHex`, `java.desEncodeToBase64String`, `java.tripleDESEncodeBase64Str`, `aesBase64DecodeToString`, `java.aesBase64DecodeToString`, `aesEncodeToBase64String`, `java.aesEncodeToBase64String` | Closed for old Core AES/DES/3DES/SM4 fixture paths | Covers `AES/CBC/PKCS5Padding`, `AES/CBC/PKCS7Padding`, `AES/CBC/ZeroPadding`, `AES/ECB/PKCS5Padding`, `DES/CBC/PKCS5Padding`, `DESede/CBC/PKCS5Padding`, `3DES/CBC/PKCS5Padding` two-key, `SM4/CBC/PKCS7Padding`, and `SM4/ECB/PKCS5Padding` Base64 encrypt/decrypt with UTF-8 key/IV strings and old Core/guarded-corpus fixtures. Object `encryptBase64(...)` is covered for the old Core search URL AES/CBC zero-padded key/IV fixture, and object `encryptHex(...)` / `decryptHex(...)` are covered for the object API fixture. `java.desEncodeToBase64String(...)` is covered for the old source Yueyou DES/ECB `_p` fixture, and `java.tripleDESEncodeBase64Str(...)` is covered for the old source QDSign 3DES/CBC/PKCS5 Base64 fixture. `createSymmetricCrypto(...)` accepts JS byte-array key/IV for the old source AES/CBC/PKCS5 Base64 key/IV fixture. AES/CBC byte-array `decrypt(...)` returns JS byte arrays and `decryptStr(...)` returns UTF-8 text for the tested imageDecode boundary fixture. This is local helper behavior only, not request execution, login state, request headers, captcha flow, or WebView behavior. |
| URI encoding | `encodeURI`, `encodeURIComponent`, and `java.*` variants | Closed for tested paths | Covers UTF-8/default paths plus ISO-8859-1/Latin1 charset overload fixtures and GBK `鬼吹灯` / `搜索` / `提交` fixture keywords for global aliases and `java.*`; full charset-table breadth is not claimed. |
| Legacy percent helpers | `escape`, `unescape` | Closed for guarded-corpus patterns | Covers `%u64CD` decoding and `escape(result)`-style `%uXXXX` output through QuickJS builtins. |
| Local time formatting | `timeFormat` and `java.timeFormat` | Closed for tested paths | Covers old Core `timeFormat(ms, format)`/`java.timeFormat(ms, format)` with deterministic UTC+8 output and Legado `java.timeFormat(ms)` default `yyyy/MM/dd HH:mm` pattern. |
| Time UTC formatting | `timeFormatUTC`, `java.timeFormatUTC` | Closed for fixture path | Covers `yyyy`, `MM`, `dd`, `HH`, `mm`, `ss` token set and offset millis. |
| Chapter-number normalization | `toNumChapter` and `java.toNumChapter` | Closed for tested paths | Covers Legado `(第)(.+?)(章)` extraction, full-width digits, Chinese numerals, and pass-through when no title-number pattern matches. |
| HTML text formatting | `htmlFormat`, `java.htmlFormat` | Closed for fixture path | Covers tag stripping, block line breaks, and common HTML entities for the old Core fixture. |
| URL resolution | `toURL`, `java.toURL` | Closed for fixture paths | Covers `String(toURL(...))`, `host`, `origin`, `pathname`, basic `searchParams`, trimmed absolute inputs, root-relative paths, and Foundation/Java-style query-only relative URLs preserving the base file path. |
| Debug log helper | `log` and `java.log` | Closed for Core log-capture path | Returns the input message and records it through reader-js console capture. No stdout or host UI side effect is claimed. |
| Toast intent helpers | `toast`, `longToast`, and `java.*` variants | Closed for Core intent-marker path | Returns empty string and records marker/message/duration through reader-js console capture. No host UI side effect is claimed. |
| Refresh TOC intent helper | `refreshTocUrl` and `java.refreshTocUrl` | Closed for Core intent-marker path | Returns empty string and records `java.refreshTocUrl() requested` through reader-js console capture. No host refresh side effect is claimed. |
| JS type names | `logType`, `java.logType` | Closed for fixture path | Covers old Core object -> `object` and array -> `array`, plus stable null/string primitive names. No logging side effect is claimed. |
| Random UUID | `randomUUID` and `java.randomUUID` | Closed for UUID v4 shape | Covers Legado lowercase UUID string shape and successive-call difference. Does not claim host crypto entropy strength. |
| Controlled WebView UA | `getWebViewUA`, `java.getWebViewUA` | Closed for fixture path | Returns the old Core deterministic default UA. Does not read host/platform WebView state. |
| Host callback routing | `ajax`, `ajaxAll`, `getSource`, and `java.*` callback routes | Boundary closed | Proves routing to host callback registry only, including timeout/cancel observation after synchronous stubs return; does not implement host network/WebView/storage behavior. |
| Chinese conversion fixture | `t2s`, `s2t`, and `java.*` variants | Closed for old Core postprocessor fixture path | Covers `國家圖書館學會 -> 国家图书馆学会` and `中国图书馆 -> 中國圖書館`; broader dictionary parity remains partial. |

## Partially Closed

| Capability | Current coverage | Remaining gap |
| --- | --- | --- |
| String/byte charset overloads | UTF-8 default paths, ISO-8859-1/Latin1 round trip, `bytesToStr` arrays/hex strings, global `strToBytes` alias, and GBK `鬼吹灯` / `搜索` / `提交` fixture keywords are covered | Full charset-table parity is not broadly covered; GBK/GB18030 encode/decode remains fixture-subset level. |
| Base64 charset | ISO-8859-1 fixture plus GBK `小说`, `鬼吹灯`, `搜索`, and `提交` fixture keywords covered | Full GBK/GB18030 table is not implemented; current GBK path is fixture-sample level. |
| Symmetric crypto | AES/CBC, AES/ECB, DES/CBC, DES/ECB, DESede/3DES CBC, DESede/3DES ECB, SM4/CBC, and SM4/ECB with PKCS5/PKCS7/ZeroPadding/NoPadding are implemented for string/Base64/hex fixture paths; object `encryptBase64(...)` is covered for the old Core AES/CBC search URL fixture; object `encryptHex(...)` / `decryptHex(...)` are covered for the object API fixture; `java.desEncodeToBase64String(...)` is covered for the old source Yueyou DES/ECB `_p` fixture; `java.tripleDESEncodeBase64Str(...)` is covered for the old source QDSign fixture; AES/CBC byte-array key/IV is covered for the old source Base64 key/IV fixture; `java.aesBase64DecodeToString` is covered for AES/CBC, DES/CBC, DESede/CBC, and SM4/CBC fixtures; `java.aesEncodeToBase64String` is covered for the guarded-corpus AES/ECB fixture; AES/CBC byte-array `decrypt(...)` / `decryptStr(...)` and `Buffer.concat(...)` are covered for imageDecode-style byte-array boundary fixtures | Global `tripleDESEncodeBase64Str`, non-UTF-8 plaintext decoding through `decryptStr(...)`, DES/3DES decode or alternate aliases, and broader transformation behavior remain unclaimed. |
| HTML formatting | Old Core text fixture covered | Full Legado `HtmlFormatter.formatKeepImg` behavior, especially image-preserving details, is not fully reproduced. |
| URL parsing | Old Core trimmed absolute, root-relative, and query-only relative fixtures covered | Full Java `URL(base, relative)` equivalence is not proven for every edge case. |
| Traditional/simplified Chinese conversion | `t2s`, `s2t`, and `java.*` variants cover a deterministic common-character subset plus the old Core postprocessor fixture path | Full Legado quick-transfer/OpenCC-style dictionary parity is not implemented. |

## Missing Pure Utility Closures

No standalone pure utility closure remains listed here at this save point.
Future rounds should take the next smallest item from `Partially Closed` and
close it with local old-behavior evidence, a minimal failing test,
implementation, and scoped test evidence.

## Excluded Or Host-Bound For This Goal

Do not mark these complete from `reader-js` pure helpers alone:

- Real network execution: `get`, `post`, `connect`, `ajax`
- WebView/browser/captcha/UI: `webView`, `webViewGetSource`, `startBrowser`,
  `startBrowserAwait`, `getVerificationCode`, `openUrl`
- Real image/body byte acquisition, decompression, and rendering stay host-owned;
  `reader-js` only covers local helper transforms once bytes are already in JS.
- File/archive/font/platform identity APIs
- Toast/UI side effects unless explicitly implemented as deterministic no-op
- ABI, protocol, bindings, storage, host app behavior

## Operating Rule For Future Rounds

Each next round should choose one item from `Partially Closed` unless a newly
discovered pure utility closure is added back to `Missing Pure Utility
Closures`, then:

1. Reconfirm current code and local Legado/old Reader-Core behavior.
2. Add the smallest failing test.
3. Implement only inside `crates/reader-js` for route 3.
4. Run focused test, `cargo test -p reader-js --quiet`, formatting, and
   `git diff --check -- crates/reader-js`.
5. Update this ledger if the status changes.
