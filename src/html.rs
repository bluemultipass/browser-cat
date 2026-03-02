// HTML head/body parsing and plain-text-to-HTML conversion.
// Ported from bcat's lib/bcat/html.rb by Ryan Tomayko.

// ── HeadParser ────────────────────────────────────────────────────────────────

/// Incrementally parses an HTML stream, separating the `<head>` from the
/// `<body>`. Feed chunks via [`HeadParser::feed`] until [`HeadParser::complete`]
/// returns true, then call [`HeadParser::head`] and [`HeadParser::take_body`].
///
/// Also detects whether the input is HTML at all: if the first non-whitespace
/// character is not `<` the document is treated as plain text.
#[derive(Debug, Default)]
pub struct HeadParser {
    buf: String,
    /// Accumulated `<head>` inner content (script/style/meta/title/link/base).
    head_parts: Vec<String>,
    /// Everything from the first body character onward.
    body_start: Option<String>,
    /// Whether we've determined the input is HTML.
    is_html: Option<bool>,
}

impl HeadParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next chunk of input. Returns `true` once the body has started
    /// (i.e. [`HeadParser::complete`] would return `true`).
    pub fn feed(&mut self, data: &str) -> bool {
        if let Some(body) = self.body_start.as_mut() {
            // Already complete — accumulate in body directly.
            body.push_str(data);
            return true;
        }
        self.buf.push_str(data);
        self.parse();
        self.body_start.is_some()
    }

    /// True once the first body character has been seen.
    pub fn complete(&self) -> bool {
        self.body_start.is_some()
    }

    /// True if the input looks like HTML (first non-whitespace char is `<`).
    pub fn is_html(&self) -> bool {
        self.is_html.unwrap_or(false)
    }

    /// Inner content of `<head>` (stripped of structural tags like DOCTYPE,
    /// `<html>`, `<head>`).
    pub fn head(&self) -> String {
        self.head_parts.join("\n")
    }

    /// Take and return all body content seen so far, leaving the parser empty.
    /// The caller is responsible for injecting a `<body>` wrapper if needed.
    pub fn take_body(&mut self) -> String {
        self.body_start.take().unwrap_or_default()
    }

    // ── internal ──────────────────────────────────────────────────────────────

    fn parse(&mut self) {
        loop {
            // Determine html-ness from first non-whitespace character.
            if self.is_html.is_none() {
                let trimmed = self.buf.trim_start();
                if trimmed.is_empty() {
                    return; // need more data
                }
                self.is_html = Some(trimmed.starts_with('<'));
            }

            if !self.is_html.unwrap_or(false) {
                // Plain text — everything is body.
                let body = std::mem::take(&mut self.buf);
                self.body_start = Some(body);
                return;
            }

            // Skip pure whitespace at the start of the buffer.
            let trimmed_start = self.buf.len() - self.buf.trim_start().len();
            if trimmed_start > 0 {
                self.buf = self.buf[trimmed_start..].to_string();
            }

            if self.buf.is_empty() {
                return;
            }

            // Try to consume a head-level tag from the buffer front.
            if let Some(consumed) = self.try_consume_head_tag() {
                if !consumed.trim().is_empty() {
                    self.head_parts.push(consumed);
                }
                continue;
            }

            // No head tag at the front — the rest is body.
            let body = std::mem::take(&mut self.buf);
            self.body_start = Some(body);
            return;
        }
    }

    /// Try to match and remove a head-level construct from the front of `buf`.
    /// Returns `Some(matched_text)` on success, `None` if no head tag is found.
    fn try_consume_head_tag(&mut self) -> Option<String> {
        let buf = &self.buf;

        // DOCTYPE
        if buf.to_ascii_uppercase().starts_with("<!DOCTYPE") {
            if let Some(end) = buf.find('>') {
                self.buf = self.buf[end + 1..].to_string();
                return Some(String::new()); // discard structural tag
            }
            return None; // incomplete, need more data
        }

        // <html ...> or </html>  — structural, discard
        if let Some(rest) = buf.strip_prefix("<html")
            && rest.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/')
        {
            if let Some(end) = buf.find('>') {
                self.buf = self.buf[end + 1..].to_string();
                return Some(String::new());
            }
            return None;
        }
        if buf.starts_with("</html") {
            if let Some(end) = buf.find('>') {
                self.buf = self.buf[end + 1..].to_string();
                return Some(String::new());
            }
            return None;
        }

        // <head> or </head> — structural, discard
        if let Some(rest) = buf.strip_prefix("<head")
            && rest.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/')
        {
            if let Some(end) = buf.find('>') {
                self.buf = self.buf[end + 1..].to_string();
                return Some(String::new());
            }
            return None;
        }
        if buf.starts_with("</head") {
            if let Some(end) = buf.find('>') {
                self.buf = self.buf[end + 1..].to_string();
                return Some(String::new());
            }
            return None;
        }

        // Head content tags we preserve: title, script, style, meta, link, base
        for tag in &["title", "script", "style", "meta", "link", "base"] {
            let open = format!("<{}", tag);
            if buf.to_ascii_lowercase().starts_with(&open) {
                let rest = &buf[open.len()..];
                if rest.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
                    // Self-closing or paired tag — find the end.
                    let close = format!("</{}>", tag);
                    if let Some(end) = buf.to_ascii_lowercase().find(&close) {
                        let full_end = end + close.len();
                        let matched = self.buf[..full_end].to_string();
                        self.buf = self.buf[full_end..].to_string();
                        return Some(matched);
                    }
                    // Self-closing `<meta ... />` or `<link ... />`
                    if let Some(end) = buf.find("/>") {
                        let full_end = end + 2;
                        let matched = self.buf[..full_end].to_string();
                        self.buf = self.buf[full_end..].to_string();
                        return Some(matched);
                    }
                    // `<meta ...>` without self-close
                    if let Some(end) = buf.find('>') {
                        let full_end = end + 1;
                        let matched = self.buf[..full_end].to_string();
                        self.buf = self.buf[full_end..].to_string();
                        return Some(matched);
                    }
                    return None; // incomplete
                }
            }
        }

        // Comments <!-- ... --> in head
        if buf.starts_with("<!--") {
            if let Some(end) = buf.find("-->") {
                let full_end = end + 3;
                let matched = self.buf[..full_end].to_string();
                self.buf = self.buf[full_end..].to_string();
                return Some(matched);
            }
            return None;
        }

        None
    }
}

// ── TextFilter ────────────────────────────────────────────────────────────────

/// Wraps plain-text chunks in `<pre>` / `</pre>` HTML.
///
/// Produces the opening `<pre>` on the first chunk and `</pre>` only when
/// [`TextFilter::finish`] is called.
pub struct TextFilter {
    opened: bool,
}

impl TextFilter {
    pub fn new() -> Self {
        Self { opened: false }
    }

    /// Convert a plain-text chunk to HTML. HTML-escapes entities.
    pub fn filter(&mut self, chunk: &str) -> String {
        let escaped = html_escape::encode_text(chunk).into_owned();
        if !self.opened {
            self.opened = true;
            format!("<pre>{}", escaped)
        } else {
            escaped
        }
    }

    /// Returns the closing `</pre>` tag (call once at end of stream).
    pub fn finish(&self) -> &'static str {
        if self.opened { "</pre>" } else { "" }
    }
}

impl Default for TextFilter {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── HeadParser ──

    #[test]
    fn detects_plain_text() {
        let mut p = HeadParser::new();
        p.feed("hello world");
        assert!(!p.is_html());
        assert!(p.complete());
        assert_eq!(p.take_body(), "hello world");
    }

    #[test]
    fn detects_html() {
        let mut p = HeadParser::new();
        p.feed("<p>hello</p>");
        assert!(p.is_html());
    }

    #[test]
    fn strips_doctype() {
        let mut p = HeadParser::new();
        p.feed("<!DOCTYPE html>\n<p>body</p>");
        assert!(p.is_html());
        assert!(p.complete());
        assert!(!p.head().contains("DOCTYPE"));
        assert!(p.take_body().contains("<p>body</p>"));
    }

    #[test]
    fn strips_html_head_tags() {
        let mut p = HeadParser::new();
        p.feed("<html><head></head><body><p>content</p></body></html>");
        assert!(p.complete());
        assert!(p.take_body().contains("<p>content</p>"));
    }

    #[test]
    fn preserves_head_content_tags() {
        let mut p = HeadParser::new();
        p.feed(
            "<html><head><title>My Page</title><style>body{}</style></head><body>hi</body></html>",
        );
        assert!(p.complete());
        let head = p.head();
        assert!(head.contains("<title>My Page</title>"));
        assert!(head.contains("<style>body{}</style>"));
    }

    #[test]
    fn fragment_with_no_head() {
        let mut p = HeadParser::new();
        p.feed("<p>just a fragment</p>");
        assert!(p.is_html());
        assert!(p.complete());
        assert!(p.take_body().contains("<p>just a fragment</p>"));
    }

    #[test]
    fn whitespace_before_html() {
        let mut p = HeadParser::new();
        p.feed("  \n  <p>text</p>");
        assert!(p.is_html());
    }

    // ── TextFilter ──

    #[test]
    fn text_filter_wraps_in_pre() {
        let mut f = TextFilter::new();
        let out = f.filter("hello");
        assert!(out.starts_with("<pre>"));
        assert!(out.contains("hello"));
    }

    #[test]
    fn text_filter_escapes_entities() {
        let mut f = TextFilter::new();
        let out = f.filter("<b>&</b>");
        assert!(out.contains("&lt;b&gt;"));
        assert!(out.contains("&amp;"));
    }

    #[test]
    fn text_filter_no_double_pre() {
        let mut f = TextFilter::new();
        let a = f.filter("first");
        let b = f.filter("second");
        assert!(a.starts_with("<pre>"));
        assert!(!b.starts_with("<pre>"));
    }

    #[test]
    fn text_filter_finish() {
        let mut f = TextFilter::new();
        f.filter("x");
        assert_eq!(f.finish(), "</pre>");
    }
}
