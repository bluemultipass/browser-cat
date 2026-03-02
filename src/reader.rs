/// Multi-source streaming reader with format sniffing.
///
/// Ported from bcat's lib/bcat/reader.rb by Ryan Tomayko.
use std::io::{self, Read};
use std::process::{Command, Stdio};

const CHUNK: usize = 16 * 1024; // 16 KiB read buffer

// ── Input format ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Html,
    Text,
}

// ── Source ────────────────────────────────────────────────────────────────────

/// A single input source (stdin, a file path, or a command to run).
#[derive(Debug, Clone)]
pub enum Source {
    Stdin,
    File(String),
    Command(Vec<String>),
}

// ── Reader ────────────────────────────────────────────────────────────────────

/// ARGF-style multi-source streaming reader.
///
/// Reads from stdin, one or more files, or a child process's stdout.
/// Uses raw `read()` calls (not line-buffered) to enable true streaming.
pub struct Reader {
    sources: Vec<Source>,
    /// Explicit format override; if `None`, sniff from the first chunk.
    forced_format: Option<Format>,
    /// The format detected (or overridden) after the first chunk.
    detected_format: Option<Format>,
}

impl Reader {
    pub fn new(sources: Vec<Source>, forced_format: Option<Format>) -> Self {
        Self {
            sources,
            forced_format,
            detected_format: None,
        }
    }

    /// Convenience: build sources from CLI args (empty args → stdin).
    /// In command mode (`-c`) treat args as a shell command.
    pub fn from_args(args: &[String], command_mode: bool) -> Self {
        let sources = if command_mode {
            vec![Source::Command(args.to_vec())]
        } else if args.is_empty() {
            vec![Source::Stdin]
        } else {
            args.iter()
                .map(|a| {
                    if a == "-" {
                        Source::Stdin
                    } else {
                        Source::File(a.clone())
                    }
                })
                .collect()
        };
        Self::new(sources, None)
    }

    /// The detected (or forced) input format. Only valid after at least one
    /// call to [`Reader::read_chunks`] or [`Reader::sniff`].
    pub fn format(&self) -> Option<Format> {
        self.forced_format.or(self.detected_format)
    }

    /// Iterate over all chunks from all sources, calling `f` for each.
    /// The first chunk triggers format sniffing if no format was forced.
    pub fn read_chunks<F>(&mut self, mut f: F) -> io::Result<()>
    where
        F: FnMut(&[u8], Format),
    {
        let sources = std::mem::take(&mut self.sources);
        for source in &sources {
            self.read_source(source, &mut f)?;
        }
        self.sources = sources;
        Ok(())
    }

    fn read_source<F>(&mut self, source: &Source, f: &mut F) -> io::Result<()>
    where
        F: FnMut(&[u8], Format),
    {
        match source {
            Source::Stdin => self.read_reader(&mut io::stdin(), f),
            Source::File(path) => {
                let mut file = std::fs::File::open(path)?;
                self.read_reader(&mut file, f)
            }
            Source::Command(args) => {
                let (prog, rest) = args.split_first().expect("empty command");
                let mut child = Command::new(prog)
                    .args(rest)
                    .stdout(Stdio::piped())
                    .spawn()?;
                let mut stdout = child.stdout.take().expect("no stdout");
                self.read_reader(&mut stdout, f)?;
                child.wait()?;
                Ok(())
            }
        }
    }

    fn read_reader<R: Read, F>(&mut self, reader: &mut R, f: &mut F) -> io::Result<()>
    where
        F: FnMut(&[u8], Format),
    {
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let chunk = &buf[..n];
            let fmt = self.ensure_format(chunk);
            f(chunk, fmt);
        }
        Ok(())
    }

    /// Determine and cache the format, peeking at `chunk` if needed.
    fn ensure_format(&mut self, chunk: &[u8]) -> Format {
        if let Some(fmt) = self.forced_format.or(self.detected_format) {
            return fmt;
        }
        let detected = sniff(chunk);
        self.detected_format = Some(detected);
        detected
    }
}

/// Peek at the first chunk to determine whether it looks like HTML.
/// Returns `Format::Html` if the first non-whitespace byte is `<`.
pub fn sniff(chunk: &[u8]) -> Format {
    let first_nonws = chunk.iter().find(|&&b| !b.is_ascii_whitespace());
    if first_nonws == Some(&b'<') {
        Format::Html
    } else {
        Format::Text
    }
}

// ── TeeFilter ─────────────────────────────────────────────────────────────────

/// Wraps a chunk-producing closure and also writes each chunk to stdout.
/// Used in `btee` mode.
pub struct TeeFilter<W: io::Write> {
    out: W,
}

impl<W: io::Write> TeeFilter<W> {
    pub fn new(out: W) -> Self {
        Self { out }
    }

    /// Write `chunk` to the tee output and return it unchanged.
    pub fn filter<'a>(&mut self, chunk: &'a [u8]) -> &'a [u8] {
        let _ = self.out.write_all(chunk); // best-effort
        chunk
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_html() {
        assert_eq!(sniff(b"<html>"), Format::Html);
        assert_eq!(sniff(b"  \n<p>"), Format::Html);
        assert_eq!(sniff(b"<!DOCTYPE"), Format::Html);
    }

    #[test]
    fn sniff_text() {
        assert_eq!(sniff(b"hello"), Format::Text);
        assert_eq!(sniff(b"  plain text"), Format::Text);
        assert_eq!(sniff(b""), Format::Text);
    }

    #[test]
    fn reader_stdin_like_cursor() {
        use std::io::Cursor;

        let data = b"hello world";
        let mut cursor = Cursor::new(data);
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        let mut fmt_seen = None;

        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = cursor.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            let chunk = buf[..n].to_vec();
            if fmt_seen.is_none() {
                fmt_seen = Some(sniff(&chunk));
            }
            chunks.push(chunk);
        }

        assert_eq!(fmt_seen, Some(Format::Text));
        assert_eq!(chunks.concat(), data);
    }

    #[test]
    fn reader_from_args_empty_is_stdin() {
        let r = Reader::from_args(&[], false);
        assert!(matches!(r.sources[0], Source::Stdin));
    }

    #[test]
    fn reader_from_args_dash_is_stdin() {
        let r = Reader::from_args(&["-".to_string()], false);
        assert!(matches!(r.sources[0], Source::Stdin));
    }

    #[test]
    fn reader_from_args_command_mode() {
        let r = Reader::from_args(&["echo".to_string(), "hi".to_string()], true);
        assert!(matches!(&r.sources[0], Source::Command(args) if args[0] == "echo"));
    }

    #[test]
    fn tee_filter_passthrough() {
        let mut captured = Vec::new();
        let mut tee = TeeFilter::new(&mut captured);
        let data = b"test data";
        let out = tee.filter(data);
        assert_eq!(out, data);
        assert_eq!(captured, data);
    }
}
