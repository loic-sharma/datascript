# Web Runtime & Local Server API

This document specifies the JSON/HTTP and WebSocket contract exposed by `ds ui`
(`datascript-server`, M4-01) and shared with the browser front end (`web/`, M4-02…M4-04) and the
in-browser WASM runtime (`@datascript/wasm`, M5-02/M5-03).

All JSON responses carry `"version": 1`. Bump it on any breaking change; front ends check it.
All diagnostic source positions use **UTF-16 code units** (D-025), matching JavaScript strings and
`datascript-diagnostics`'s `JsonReport` (version 1).

## Security model (`ds ui`)

1. **Loopback only.** `ds ui` binds exclusively to `127.0.0.1` on an ephemeral port (`--port 0` by
   default).
2. **Per-launch session token.** Every launch generates a 256-bit cryptographically random hex
   token and prints `http://127.0.0.1:<port>/?token=<token>`.
   - Static front-end assets (`/`, `/assets/*`) are served without requiring an auth header so the
     browser can open the launch URL, read `token` from `location.search`, remove it from the
     address bar (`history.replaceState`), and send it on API requests.
   - Every `/api/*` request (HTTP and WebSocket upgrade) must present the token in one of:
     - `Authorization: Bearer <token>` (preferred for `fetch`)
     - `X-DataScript-Token: <token>`
     - `?token=<token>` query parameter (for browser `WebSocket` and download links)
   - Missing or invalid tokens fail with `401 Unauthorized`.
3. **Origin & Host checks.**
   - When an `Origin` header is present on `/api/*`, it must match `http://127.0.0.1:<port>`,
     `http://localhost:<port>`, `http://[::1]:<port>`, or the `--dev-proxy` origin (and its
     `localhost`/`127.0.0.1` twin). Any other origin fails with `403 Forbidden`.
   - When a `Host` header is present on `/api/*`, its hostname must be `127.0.0.1`, `localhost`,
     or `[::1]` (anti-DNS-rebinding); otherwise `403 Forbidden`.
4. **Root directory sandbox (D-016).**
   - Every source a query resolves (file path, glob, reader call, or registered host table) must
     canonicalize inside the UI root directory after resolving `..` and symlinks (including
     dangling symlinks and escaping glob prefixes). Paths outside the root fail with `DS3007`.
   - `--allow-outside-root` disables query sandboxing.
   - `GET /api/files` always confines directory listing strictly inside the root directory and
     never exposes symlinks pointing outside it.

## Error envelope

HTTP-level errors (authentication, origin/host rejection, malformed requests, path traversal on
`/api/files`, unknown `/api/*` routes) return:

```json
{
  "error": {
    "code": "missing_token",
    "message": "missing session token; pass `Authorization: Bearer <token>`, `X-DataScript-Token`, or `?token=<token>`"
  }
}
```

DataScript language, type, source, sandbox, and runtime errors are returned as standard
`JsonDiagnostic` objects (see `datascript-diagnostics/src/json.rs`).

## Shared data types

### `ColumnInfo`

```json
{ "name": "amount", "type": "real" }
```

`type` is the DataScript type name (`bool`, `int`, `long`, `real`, `string`, `datetime`,
`timespan`, `dynamic`, `guid`, `decimal`), or `arrow:<DataType>` for projection-only Arrow types
(LANGUAGE.md §5.1).

### `RenderSpec`

Camel-case JSON produced by `datascript-compiler` (ARCHITECTURE.md § Render layer, D-047). Absent
when a query has no `render` operator.

### `JsonDiagnostic` & `JsonSpan`

Matches `datascript-diagnostics` JSON format version 1 (UTF-16 offsets and 1-based line/column).

### Row limits & truncation (D-062)

UI runtimes default to **10,000 rows per query** (`DEFAULT_UI_ROW_LIMIT`). When a query produces
more rows than the effective limit, execution stops (cancelling the engine stream), `truncated` is
set to `true`, and an info diagnostic **`DS4060`** (`result truncated to N rows`) is emitted
pointing at the query statement with help suggesting export or narrowing with `where`/`take`.

Pass `"rowLimit": 0` to disable the limit on a run, or call `POST /api/export` to stream the full
untruncated result.

## HTTP Endpoints

### `GET /api/info`

Returns server configuration and capabilities.

Response `200 OK`:

```json
{
  "version": 1,
  "root": "/path/to/data",
  "sandboxed": true,
  "defaultRowLimit": 10000,
  "engine": "datafusion",
  "tables": ["orders"],
  "exportFormats": ["csv", "json", "ndjson", "arrow"]
}
```

### `POST /api/check`

Checks a DataScript program without executing it.

Request:

```json
{
  "query": "from 'orders.csv' | take 5",
  "filename": "query.ds"
}
```

`filename` is optional (defaults to `"query.ds"`).

Response `200 OK`:

```json
{
  "version": 1,
  "hasErrors": false,
  "diagnostics": [],
  "queries": [
    {
      "index": 0,
      "span": {
        "file": "query.ds",
        "start": 0,
        "end": 26,
        "line": 1,
        "column": 1,
        "endLine": 1,
        "endColumn": 27
      },
      "render": { "kind": "table" }
    }
  ]
}
```

`queries` lists each query statement's 0-based index, UTF-16 span, and optional `RenderSpec` when
checking succeeds without errors (used by the editor for Ctrl/Cmd+Enter statement selection).

### `POST /api/run`

Checks, plans, and runs a program (or one selected query statement), enforcing the row limit.

Request:

```json
{
  "query": "from 'orders.csv' | take 5",
  "filename": "query.ds",
  "statement": 0,
  "rowLimit": 10000,
  "format": "json"
}
```

- `filename`: optional (default `"query.ds"`).
- `statement`: optional 0-based query statement index. The whole program is still checked; only the
  selected statement runs. Out-of-range index returns `400 Bad Request` (`invalid_statement`).
- `rowLimit`: optional. Omit for the server default (`10000`); `0` disables the limit; positive
  integer caps each query result.
- `format`: `"json"` (default) or `"arrow"`.

Response `200 OK`:

```json
{
  "version": 1,
  "hasErrors": false,
  "diagnostics": [],
  "results": [
    {
      "statement": 0,
      "span": { "file": "query.ds", "start": 0, "end": 26, "line": 1, "column": 1, "endLine": 1, "endColumn": 27 },
      "columns": [
        { "name": "orderId", "type": "string" },
        { "name": "amount", "type": "real" }
      ],
      "rowCount": 2,
      "truncated": false,
      "render": { "kind": "barchart", "x": "orderId", "y": ["amount"], "legend": "visible", "stacking": "default" },
      "rows": [
        { "orderId": "o-1", "amount": 10.5 },
        { "orderId": "o-2", "amount": 25.0 }
      ]
    }
  ]
}
```

- When `format` is `"arrow"`, `"rows"` is omitted and `"arrowIpc"` holds the base64-encoded Arrow
  IPC file bytes (`arrow_ipc::writer::FileWriter`, readable by Apache Arrow JS `tableFromIPC`).
- Compile, sandbox (`DS3007`), and runtime (`DS4003`, `DS3008`, …) diagnostics are returned in
  `"diagnostics"` with `"hasErrors": true`.
- Truncated results set `"truncated": true` and include a `DS4060` info diagnostic.

### `POST /api/export`

Streams the **full, untruncated** result of a single query statement using
`datascript_engine::writers` and Arrow IPC (D-062).

Request:

```json
{
  "query": "from 'orders.csv'",
  "filename": "query.ds",
  "statement": 0,
  "format": "csv"
}
```

- `format`: `"csv"` (default), `"json"`, `"ndjson"`, or `"arrow"`.
- `statement`: required when the program has more than one query statement (`400 Bad Request`
  `multiple_statements` otherwise).

Responses:
- `200 OK` streaming response body with `Content-Disposition: attachment; filename="result.<ext>"`
  and `Content-Type`:
  - `csv`: `text/csv; charset=utf-8`
  - `json`: `application/json; charset=utf-8`
  - `ndjson`: `application/x-ndjson; charset=utf-8`
  - `arrow`: `application/vnd.apache.arrow.file`
- `422 Unprocessable Entity` with `{ "version": 1, "hasErrors": bool, "diagnostics": [...] }` if
  checking fails, there is no query to run (`DS0004`), or reading the first batch fails.
- Closing the HTTP connection cancels the underlying query stream.

### `GET /api/files`

Lists subdirectories and recognized data files under the root directory (or under `?dir=<relpath>`).

Query parameters:
- `dir`: optional path relative to the root directory (default `""`). Any absolute path, `..`
  segment, or symlink escaping the root returns `403 Forbidden` (`outside_root`).

Response `200 OK`:

```json
{
  "version": 1,
  "dir": "",
  "entries": [
    { "name": "events", "path": "events", "kind": "directory" },
    { "name": "orders.csv", "path": "orders.csv", "kind": "file", "format": "csv", "size": 1234 }
  ],
  "tables": ["orders"]
}
```

Entries are sorted deterministically: directories first, then recognized data files, each
alphabetically by `name`. Hidden entries (leading `.`), unrecognized file extensions, and symlinks
resolving outside the root are omitted.

### `GET /api/schema`

Inspects the columns and DataScript types of a file, glob, or registered host table.

Query parameters:
- `source`: required file path, glob, or host table name.
- `kind`: optional `"file"` or `"table"`. When omitted, names matching a registered host table are
  resolved as tables and everything else as a file/glob path.

Responses:
- `200 OK`:
  ```json
  {
    "version": 1,
    "source": "orders.csv",
    "columns": [
      { "name": "orderId", "type": "string" },
      { "name": "amount", "type": "real" }
    ]
  }
  ```
- `403 Forbidden` (`DS3007` outside sandbox), `404 Not Found` (`DS3001` / `DS1002`), or
  `400 Bad Request` (`DS3002` / `DS3004` / `DS3005` / `DS3008` / `DS3009`):
  ```json
  {
    "version": 1,
    "source": "../outside.csv",
    "diagnostics": [ ... ]
  }
  ```

### Reserved endpoints (M4-04)

`POST /api/complete`, `POST /api/hover`, and `POST /api/signature-help` are added in M4-04, backed
by `datascript_compiler::ide`.

## WebSocket Streaming & Cancellation (`GET /api/ws`)

Connect to `ws://127.0.0.1:<port>/api/ws?token=<token>` (valid `Origin` enforced on upgrade).
Frames are UTF-8 JSON objects multiplexed by a client-chosen string `id`.

### Client → Server

Run a program (backed by `Session::start` / `RunStream`):

```json
{
  "type": "run",
  "id": "req-1",
  "query": "from 'orders.csv' | sort by amount desc",
  "filename": "query.ds",
  "statement": 0,
  "rowLimit": 10000,
  "format": "json"
}
```

Cancel an in-flight run (aborts the task and drops `RunStream` immediately):

```json
{ "type": "cancel", "id": "req-1" }
```

Closing the WebSocket aborts all in-flight runs for that connection immediately.

### Server → Client

- `diagnostics` (check warnings/errors, runtime errors, or `DS4060` truncation info):
  ```json
  { "type": "diagnostics", "id": "req-1", "version": 1, "hasErrors": false, "diagnostics": [...] }
  ```
- `queryStart` (one per query statement):
  ```json
  {
    "type": "queryStart",
    "id": "req-1",
    "statement": 0,
    "span": { ... },
    "columns": [{ "name": "orderId", "type": "string" }],
    "render": { ... }
  }
  ```
- `batch` (streamed as the engine produces batches; carries `rows` for `"json"` or `arrowIpc`
  base64 Arrow IPC file bytes for `"arrow"`):
  ```json
  { "type": "batch", "id": "req-1", "statement": 0, "rowCount": 128, "rows": [...] }
  ```
- `queryEnd`:
  ```json
  { "type": "queryEnd", "id": "req-1", "statement": 0, "rowCount": 128, "truncated": false }
  ```
- `cancelled` (acknowledges `cancel`):
  ```json
  { "type": "cancelled", "id": "req-1" }
  ```
- `done` (end of this `run` request):
  ```json
  { "type": "done", "id": "req-1", "hasErrors": false }
  ```
- `error` (protocol error such as malformed JSON, duplicate active `id`, or out-of-range
  `statement`):
  ```json
  { "type": "error", "id": "req-1", "code": "invalid_statement", "message": "..." }
  ```

## Shared front-end `Runtime` contract (M4-02 & M5-02)

`web/` talks to a TypeScript `Runtime` interface implemented by both `ServerRuntime` (calling the
endpoints above) and `WasmRuntime` (wrapping `@datascript/wasm` in a Web Worker):

- `check(query, filename?)` → `{ version, hasErrors, diagnostics, queries }`
- `run(query, options?)` → yields/returns `{ version, hasErrors, diagnostics, results }` with
  optional `statement`, `rowLimit`, `format` (`"json" | "arrow"`), and cancellation (`AbortSignal`
  → WS `cancel` or Worker termination)
- `export(query, { statement?, format })` → full untruncated `Blob` / byte stream
- `listFiles(dir?)` → `{ version, dir, entries, tables }`
- `schema(source, kind?)` → `{ version, source, columns }` or `{ version, source, diagnostics }`

M5-02 extends this document with the exact `@datascript/wasm` JavaScript method signatures
(`init`, `registerFile`, `registerTable`, injected `now`, `explain`, IDE methods).

## Front-end assets & Vite dev mode

- Production builds embed `web/dist/` into the `ds` binary via `rust-embed` and serve `index.html`
  for `/` and extensionless SPA routes.
- During front-end development, pass `--dev-proxy http://127.0.0.1:5173` to `ds ui`: non-`/api/*`
  requests are reverse-proxied to Vite while `/api/*` is served locally and accepts Vite's `Origin`.
