/// browser-cat: pipe to browser utility.
///
/// Rust port of bcat by Ryan Tomayko (https://github.com/rtomayko/bcat).
use browser_cat::{
    ansi::ansi_chunk_to_html,
    browser::Browser,
    html::{HeadParser, TextFilter},
    reader::{Format, Reader, Source, TeeFilter},
    server::{self, ServerConfig},
};
use bytes::Bytes;
use clap::Parser;
use std::io;
use std::path::Path;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "browser-cat",
    about = "Pipe to browser utility",
    long_about = "Read standard input, possibly one or more files, and write \
                  concatenated/formatted output to a browser window.\n\n\
                  When invoked as btee, also write all input back to stdout."
)]
struct Cli {
    /// Input is already HTML (passthrough)
    #[arg(short = 'H', long)]
    html: bool,

    /// Input is plain text (wrap in <pre>)
    #[arg(short, long)]
    text: bool,

    /// Open this browser instead of the system default
    #[arg(short, long, value_name = "BROWSER")]
    browser: Option<String>,

    /// Browser window title
    #[arg(short = 'T', long, value_name = "TEXT")]
    title: Option<String>,

    /// Convert ANSI escape sequences to HTML
    #[arg(short, long)]
    ansi: bool,

    /// Keep server running after browser closes (allow reload)
    #[arg(short, long)]
    persist: bool,

    /// Treat arguments as a command to run; read its stdout
    #[arg(short, long = "command")]
    command: bool,

    /// Verbose debug logging to stderr
    #[arg(short, long)]
    debug: bool,

    /// Bind host (default: 127.0.0.1)
    #[arg(long, default_value = "127.0.0.1", hide = true)]
    host: String,

    /// Bind port (default: 0 = OS-assigned)
    #[arg(long, default_value_t = 0, hide = true)]
    port: u16,

    /// Files or command arguments
    #[arg(value_name = "FILE")]
    args: Vec<String>,
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Detect btee mode: invoked as a binary whose name ends with "tee".
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let tee_mode = exe.ends_with("tee");

    let debug = cli.debug;
    macro_rules! notice {
        ($($arg:tt)*) => {
            if debug { eprintln!("browser-cat: {}", format!($($arg)*)); }
        };
    }

    // Forced format from flags.
    let forced_format = if cli.html {
        Some(Format::Html)
    } else if cli.text {
        Some(Format::Text)
    } else {
        None
    };

    // Build reader sources.
    let sources: Vec<Source> = if cli.command {
        vec![Source::Command(cli.args.clone())]
    } else if cli.args.is_empty() {
        vec![Source::Stdin]
    } else {
        cli.args
            .iter()
            .map(|a| {
                if a == "-" {
                    Source::Stdin
                } else {
                    Source::File(a.clone())
                }
            })
            .collect()
    };

    let mut reader = Reader::new(sources, forced_format);

    // Browser name: CLI flag > env var > "default".
    let browser_name = cli
        .browser
        .clone()
        .or_else(|| std::env::var("BCAT_BROWSER").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "default".into());

    notice!("browser: {}", browser_name);

    let browser = Browser::new(&browser_name);
    notice!("command: {}", browser.command());

    // Start the server.
    notice!("starting server");
    let cfg = ServerConfig {
        host: cli.host.clone(),
        port: cli.port,
        persist: cli.persist,
    };

    let handle = server::serve(cfg, |addr| {
        let cwd_name = std::env::current_dir()
            .ok()
            .and_then(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        let url = format!("http://{}:{}/{}", addr.ip(), addr.port(), cwd_name);
        notice!("url: {}", url);

        match browser.open(&url) {
            Ok(child) => {
                notice!("browser pid: {}", child.id());
                // We don't wait on the child in this version — the process
                // exits naturally when the server shuts down.
                std::mem::forget(child);
            }
            Err(e) => {
                eprintln!("browser-cat: failed to open browser: {}", e);
            }
        }
    })
    .await;

    // Emit the HTML preamble, then stream body chunks.
    let title = cli.title.clone().unwrap_or_else(|| {
        Path::new(&cli.args.first().cloned().unwrap_or_default())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "browser-cat".into())
    });

    let mut head_parser = HeadParser::new();
    let mut text_filter = TextFilter::new();
    let mut preamble_sent = false;
    let mut tee: Option<TeeFilter<io::Stdout>> = if tee_mode {
        Some(TeeFilter::new(io::stdout()))
    } else {
        None
    };

    let result = reader.read_chunks(|raw_chunk, fmt| {
        // Tee: write raw bytes to stdout before any transformation.
        if let Some(ref mut t) = tee {
            t.filter(raw_chunk);
        }

        // Convert ANSI if requested.
        let converted: Bytes = if cli.ansi {
            let s = String::from_utf8_lossy(raw_chunk);
            Bytes::from(ansi_chunk_to_html(&s))
        } else {
            Bytes::copy_from_slice(raw_chunk)
        };

        match fmt {
            Format::Html => {
                // Feed into HeadParser until it's complete, then stream body.
                let chunk_str = String::from_utf8_lossy(&converted).into_owned();
                if !head_parser.complete() {
                    head_parser.feed(&chunk_str);
                    if head_parser.complete() {
                        // Send the full preamble + initial body.
                        let preamble = build_preamble(&title, &head_parser.head());
                        handle.send(Bytes::from(preamble));
                        let body_start = head_parser.take_body();
                        // Re-init parser (we've consumed it) — use a trick:
                        // we must not call into_body again, so flush here.
                        if !body_start.is_empty() {
                            handle.send(Bytes::from(body_start));
                        }
                        preamble_sent = true;
                    }
                    // If not complete yet, buffer (head_parser holds it).
                } else {
                    if !preamble_sent {
                        let preamble = build_preamble(&title, "");
                        handle.send(Bytes::from(preamble));
                        preamble_sent = true;
                    }
                    handle.send(converted);
                }
            }
            Format::Text => {
                let chunk_str = String::from_utf8_lossy(&converted).into_owned();
                let html = text_filter.filter(&chunk_str);
                handle.send(Bytes::from(html));
            }
        }
    });

    if let Err(e) = result {
        eprintln!("browser-cat: read error: {}", e);
    }

    // Closing tags.
    let fmt = reader.format().unwrap_or(Format::Text);
    match fmt {
        Format::Text => {
            let closing = text_filter.finish();
            if !closing.is_empty() {
                handle.send(Bytes::from(closing));
            }
        }
        Format::Html => {
            // HTML input: ensure we at least sent a preamble.
            if !preamble_sent {
                let preamble = build_preamble(&title, "");
                handle.send(Bytes::from(preamble));
            }
        }
    }

    notice!("done reading; shutting down");
    handle.finish();

    // Give the server a moment to flush the response before exiting.
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
}

// ── HTML preamble ─────────────────────────────────────────────────────────────

fn build_preamble(title: &str, extra_head: &str) -> String {
    let title_escaped = html_escape::encode_text(title);
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>{title}</title>
<style>
  body {{ background:#1e1e1e; color:#d4d4d4; font-family:monospace; margin:1em; }}
  pre  {{ white-space:pre-wrap; word-wrap:break-word; }}
</style>
{extra}
</head>
<body>
"#,
        title = title_escaped,
        extra = extra_head,
    )
}
