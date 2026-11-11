# PR：新增插件 jiusi.mybatis-log

> 提给 [l0ng-ai/openidea-plugins](https://github.com/l0ng-ai/openidea-plugins) 的 PR 正文草稿。
> publisher `jiusi` 已在 registry 中，本地 `publishers/jiusi.toml` 与官方条目逐字一致，
> 所以 PR 的实际差异只有新增的 `plugins/jiusi.mybatis-log.toml`。
> 顺序：先把源码推到 `jiusi666/openidea-mybatis-log`，发 `v0.1.0` Release 并上传
> `jiusi.mybatis-log-0.1.0.oiplugin`，再跑 `python scripts/open_pr.py`。以上三步已于 2026-11-11 执行，
> PR 为 https://github.com/l0ng-ai/openidea-plugins/pull/117 。

## 标题

```
Add jiusi.mybatis-log — restore MyBatis console logs into executable SQL
```

## 正文

```markdown
### What it is

`jiusi.mybatis-log` turns MyBatis / MyBatis-Plus console logs into SQL you can paste
straight into a database client. Paste a log into the side panel (or select it in any
editor and press `Alt+Shift+M` / `Cmd+Shift+M`) and each `Preparing:` line is filled in
from its following `Parameters:` line:

    ==>  Preparing: select id, name from user where id = ? and name = ?
    ==> Parameters: 1001(Long), ACTIVE(String)

becomes

    select id, name from user where id = 1001 and name = 'ACTIVE'

It handles batches of statements, logback timestamps and logger prefixes, Java type
suffixes (numbers and booleans unquoted, strings / dates / enums quoted and escaped,
`null`, empty strings, fully qualified type names, commas inside values), and skips
`?` inside string literals and comments. Mismatched placeholder / parameter counts are
reported instead of silently producing wrong SQL. Display options (clause line breaks,
uppercase keywords, trailing semicolon, quote style, `NULL` vs `null`) only affect the
view; copied SQL stays single-line and executable.

This is deliberately a *log-restoration* tool, not a navigation plugin. The official
example plugin `openidea.mybatis` (MyBatis Navigator) already covers mapper interface ↔
XML gutter icons and Go to Definition; this plugin does not duplicate it, and no plugin
in the registry currently restores MyBatis SQL logs. The official `string-tools` example
is a set of generic plain-text commands; this is structure-aware SQL reconstruction.

Source: https://github.com/jiusi666/openidea-mybatis-log (Apache-2.0).

### Capabilities

`editor.read` (Low, required) — reads the focused editor's document and selections for
the single command `MyBatis Log: 还原选中的日志`. When there is no selection it reads the
whole open document (handy for `.log` files). Read-only; `apply_edits` is never called.

`storage` (Low, required) — the panel's input text, display options and the last 10–30
restoration inputs, on this machine only, removed on uninstall. Results are recomputed
from the saved input on activation (pure text, microseconds), so no large result blobs
are persisted.

No `project.read`, no `editor.write`, no `clipboard`, no `net:`, no `process:`, no
`fs.*`, no `vcs.*`, no `secrets`. Copying is implemented inside the sandboxed web view
by selecting a temporary textarea, so `host::copy_to_clipboard` and the clipboard API
are never used. The panel runs without network of its own; all data is pushed from the
WASM side. Nothing leaves the machine.

### Verification

- `cargo +stable-x86_64-pc-windows-gnu test` — 27 unit tests: log parsing (standard,
  logback prefixes, batches, missing `Parameters`, plain SQL, empty input), parameter
  splitting (comma inside a value, `[]`, fully qualified types, empty string, `null`),
  type rendering and quote escaping (single / double quote modes), `?` skipping inside
  literals and comments, placeholder count warnings, clause breaks, uppercase keywords,
  UTF-8 truncation, entry limits, state normalisation, options, history and view messages.
- `cargo +stable-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings` — clean.
- `cargo +stable-x86_64-pc-windows-gnu build --release --target wasm32-wasip2` — the
  shipped `dist/plugin.wasm` is a WebAssembly **component**.
- `python scripts/oiplugin.py check .` — manifest, contributions, icons, capabilities
  and component magic all pass.
- `python scripts/oiplugin.py verify dist/jiusi.mybatis-log-0.1.0.oiplugin` — digest
  recomputed, Ed25519 signature verified against publisher `jiusi`.
- `node --check` on the panel's inline script — syntax clean; light and dark layouts
  checked in a 514 px headless-Chrome screenshot.
- `python scripts/local_install.py status --project .` — no conflicting Installed /
  Linked / Dev copy of this id.

### Notes

- Built from this source with `wasm32-wasip2` + component model
  (`engines.openidea = "^1.4"`); no minified or generated-only code. The SDK source is
  vendored under `vendor/openidea-plugin` so the build is reproducible offline.
- Package: `jiusi.mybatis-log-0.1.0.oiplugin`, sha256
  `b2e21db68036fb802d7e37f5f0c2da0f1bff58db614094fdb803030955f5d13f`.
- Limits, documented in the README: no Java AST (pure text reconstruction), at most
  300 statements and 512 KiB of input per restoration, booleans emitted as `true/false`.
```
