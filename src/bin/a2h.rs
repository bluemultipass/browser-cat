/// a2h: convert ANSI/VT100 escape sequences to HTML.
///
/// Reads from stdin or one or more files, writes converted HTML to stdout.
/// Ported from bcat's bin/a2h by Ryan Tomayko.
use browser_cat::ansi::ansi_to_html;
use std::io::{self, Read, Write};
use std::{env, fs};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let sources: Vec<String> = if args.is_empty() {
        vec!["-".into()]
    } else {
        args
    };

    for source in &sources {
        let input = if source == "-" {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .expect("failed to read stdin");
            buf
        } else {
            fs::read_to_string(source).unwrap_or_else(|e| {
                eprintln!("a2h: {}: {}", source, e);
                std::process::exit(1);
            })
        };

        let html = ansi_to_html(&input);
        out.write_all(html.as_bytes())
            .expect("failed to write stdout");
    }
}
