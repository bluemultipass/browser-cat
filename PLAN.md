# browser-cat: Rust port of bcat

## Project structure

```
browser-cat/
  Cargo.toml
  src/
    lib.rs         # shared types, re-exports
    ansi.rs        # ANSI escape → HTML (used by browser-cat and a2h)
    html.rs        # HeadParser + TextFilter
    reader.rs      # multi-source streaming reader
    browser.rs     # cross-platform browser launcher
    server.rs      # HTTP server + streaming
  src/bin/
    browser-cat.rs # main binary (also handles btee mode via argv[0])
    a2h.rs         # standalone ANSI filter
```

`btee` is a symlink to `browser-cat` at install time, same as the original. `browser-cat.rs` checks `argv[0]` ends with `tee` to enable tee mode.

---

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` (derive) | CLI arg parsing |
| `tokio` (full) | async runtime |
| `axum` | HTTP server + streaming response body |
| `tokio-stream` | adapt channels into streaming bodies |
| `regex` | ANSI tokenizer |
| `html-escape` | entity-escape plain text |
| `open` | default browser launching (cross-platform) |

No crate for ANSI conversion — implement it ourselves from the Ruby source.

---

## Module breakdown

**`ansi.rs`**
- Port the `STYLES` hash as a `match` expression or `phf` static map
- 16 base colors + generate the xterm-256 palette at startup (same algorithm as Ruby)
- Tokenizer: 5 regex patterns (backspace, xterm-256, standard ANSI, malformed, text)
- Stack-based tag tracking; emit `<span style="...">` / `</span>` on reset
- Exposes: `fn ansi_to_html(input: &str) -> String` and a streaming iterator form

**`html.rs`**
- `HeadParser`: incremental state machine that buffers incoming chunks, separates `<head>` from `<body>`, detects whether input is HTML at all (first non-whitespace char is `<`)
- `TextFilter`: wraps chunks in `<pre>`, HTML-escapes entities
- `TeeFilter`: wraps a reader, writes each chunk to stdout while also yielding it — enabled in btee mode

**`reader.rs`**
- `Reader`: reads from stdin, files, or a child process (`-c` mode) — same ARGF-style multi-source behavior
- Uses `read` (not buffered line-by-line) to enable streaming
- `sniff()`: peeks at first chunk to detect HTML vs text when format not specified

**`browser.rs`**
- Static lookup table: `(OS, browser_name) → command`
- macOS: uses `open` / `open -a <App>`
- Linux: uses `xdg-open`, `firefox`, `google-chrome`, etc.
- Windows: `start` (not in original, but worth adding)
- Named browser aliases (`google-chrome` → `chrome`)
- Spawns browser as detached child process; returns the `Child` handle
- URL construction: `http://127.0.0.1:<port>/<basename-of-cwd>`

**`server.rs`**
- Bind `TcpListener` on `127.0.0.1:0`, get OS-assigned port
- Wrap in axum, single route `GET /`
- Handler receives an `mpsc::Receiver<Bytes>` and returns a `StreamBody`
- Reader runs in `spawn_blocking`, sends chunks into the channel
- Persist mode: after first request completes, keep the server alive (don't shut down)
- Builds the HTML response envelope (head, injected CSS, body open tag) from `HeadParser` output, then streams the body chunks

---

## Key design decisions

**Streaming**: `tokio::sync::mpsc::channel` bridges the blocking reader thread and the async HTTP handler. Reader pushes `Bytes` chunks; axum streams them as the response body. This preserves the "display progressively" behavior.

**Format detection**: Happens in the reader before the server starts, or on the first chunk. Since we need to know the format to build the HTTP response headers/preamble, `sniff()` peeks at the first chunk synchronously before the browser is opened.

**Persist mode**: The server holds a `Notify` or `watch` channel; in normal mode it shuts down after the first response body is fully sent. In persist mode it stays alive and re-serves the accumulated content on subsequent requests (buffer everything already sent).

**btee**: Detected via `std::env::current_exe()` name check. When enabled, each chunk sent to the HTTP server is also written to `io::stdout()`.

**`-c` mode**: Use `std::process::Command` to spawn the child; read its stdout as the input stream.

---

## Implementation order

1. `ansi.rs` + `a2h` binary — self-contained, testable in isolation
2. `html.rs` — `HeadParser` and `TextFilter`
3. `reader.rs` — multi-source reader, sniff, tee filter
4. `browser.rs` — browser table + spawn
5. `server.rs` — axum server + streaming
6. `bcat.rs` — wire everything together, CLI flags
