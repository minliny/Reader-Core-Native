# Host Replay Fixture Format (`reader-host-replay/1`)

Offline replay fixtures for the `host-replay` dev tool. Each fixture is a single
JSON file describing one recorded `host.request` interaction and the response a
real host adapter would return. The tool reads these and emits `host.complete` /
`host.error` JSON commands — **no network, no protocol-schema changes**.

> Scope: this is a **development-time** artifact format, owned by
> `tools/host-replay`. It is NOT part of the Reader-Core protocol schema. It
> only describes how to *replay* responses that the protocol already defines.

## Top-level object

| field           | type     | required | notes                                                  |
|-----------------|----------|----------|--------------------------------------------------------|
| `format`        | string   | yes      | must be `"reader-host-replay/1"`                       |
| `description`   | string   | no       | human-readable label                                   |
| `request`       | object   | yes      | recorded request (see below)                           |
| `response`      | object   | complete | recorded response; required when `outcome="complete"`  |
| `redirectChain` | array    | no       | 3xx hops; last hop's `location` becomes `finalUrl`     |
| `cookieJar`     | object   | no       | snapshot: `{ "<origin>": [Cookie, ...] }`              |
| `outcome`       | string   | no       | `"complete"` (default) or `"error"`                    |
| `error`         | object   | error    | transport error; required when `outcome="error"`       |
| `tags`          | string[] | no       | free-form labels for filtering                         |

## `request`

| field          | type    | required | notes                                                              |
|----------------|---------|----------|--------------------------------------------------------------------|
| `id`           | integer | no       | recorded `requestId` label (NOT used for matching)                 |
| `operationId`  | integer | no       | recorded `operationId` label (live op id comes from the incoming event) |
| `capability`   | string  | no       | default `"http.execute"`                                           |
| `url`          | string  | no       | request URL; matched normalized (scheme/host case-insensitive, sorted query) |
| `urlPattern`   | string  | no       | wildcard pattern (`*`/`?`); overrides `url` for matching           |
| `method`       | string  | no       | default `"GET"`; matched case-insensitively                        |
| `headers`      | object  | no       | request headers                                                    |
| `body`         | any     | no       | request body (informational)                                       |

## `response`

| field          | type    | notes                                                              |
|----------------|---------|--------------------------------------------------------------------|
| `status`       | integer | HTTP status (emitted as `result.status`)                           |
| `headers`      | object  | response headers; `Set-Cookie` may be a string or array            |
| `body`         | string  | inline text body → `result.body`                                   |
| `bodyFile`     | string  | sibling file path; read as text or base64 per `bodyEncoding`       |
| `bodyEncoding` | string  | `"text"` (default) or `"base64"`                                   |
| `bodyBase64`   | string  | raw base64 → `result.bodyBase64` (for binary; bypasses `bodyFile`) |
| `finalUrl`     | string  | explicit final URL; else last redirect `location`                 |
| `charsetHint`  | string  | optional charset hint → `result.charsetHint`                       |

Body resolution precedence: `bodyBase64` > `bodyFile` > `body`.

## `redirectChain[]`

| field        | type     | notes                                   |
|--------------|----------|-----------------------------------------|
| `status`     | integer  | must be 3xx                             |
| `location`   | string   | `Location` header value (next hop URL)  |
| `headers`    | object   | headers seen on this hop                |
| `setCookies` | string[] | `Set-Cookie` values seen on this hop    |

## `cookieJar`

```json
{
  "https://example.test": [
    {
      "name": "sid",
      "value": "abc",
      "domain": "example.test",
      "path": "/",
      "secure": true,
      "httpOnly": true,
      "expires": "2026-12-31T00:00:00Z",
      "sameSite": "Lax"
    }
  ]
}
```

Keyed by origin (`scheme://host`). Used as a snapshot and, with
`--update-jar <file>`, merged with `Set-Cookie` from the matched response.

## `error` (for `outcome="error"`)

```json
{
  "code": "HTTP_TRANSPORT_TIMEOUT",
  "message": "connect timed out",
  "retryable": true,
  "details": { "phase": "connect", "url": "https://example.test/path" }
}
```

Maps directly to `host.error` params.error. `code` is a free-form string;
the convention is to use the transport error codes from
`docs/host-app-contracts/01-network-session.md` (e.g.
`HTTP_TRANSPORT_TIMEOUT`, `HTTP_TRANSPORT_DNS`, `HTTP_TRANSPORT_TLS`,
`HTTP_TRANSPORT_CONNECT`, `HTTP_TRANSPORT_HTTP_STATUS`).

## Matching rules

When `host-replay replay` reads a `host.request` event from stdin, it matches
against loaded fixtures by:

1. **Capability** must match exactly.
2. If `urlPattern` is set: wildcard-match (`*` = any sequence, `?` = one char)
   against the incoming `params.url`.
3. Otherwise: normalized exact match on `url` + case-insensitive `method`.

`request.id` and `request.operationId` are **not** used for matching — they are
recorded labels. The live `operationId` from the incoming event is echoed into
the emitted command.

## Emitted output

- `outcome="complete"` → `host.complete` command envelope (protocolVersion 1).
- `outcome="error"` → `host.error` command envelope (protocolVersion 1).

The output is one JSON object per line (or pretty-printed with `--pretty`),
suitable for piping into a Core runtime's stdin or for diff-based assertions.
