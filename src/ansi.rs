/// ANSI/VT100 escape sequence to HTML converter.
///
/// Ported from bcat's lib/bcat/ansi.rb by Ryan Tomayko.
use regex::Regex;
use std::sync::OnceLock;

// ── Colour palette ────────────────────────────────────────────────────────────

/// Returns the CSS hex colour for a given xterm-256 palette index.
fn xterm256_color(index: u8) -> String {
    let i = index as u32;
    if i < 16 {
        // First 16: named Linux console colours (same order as ANSI_COLORS below)
        ANSI_COLORS[i as usize].to_string()
    } else if i < 232 {
        // 6×6×6 colour cube
        let idx = i - 16;
        let b = idx % 6;
        let g = (idx / 6) % 6;
        let r = idx / 36;
        let to_byte = |v: u32| if v == 0 { 0u32 } else { v * 40 + 55 };
        format!("#{:02x}{:02x}{:02x}", to_byte(r), to_byte(g), to_byte(b))
    } else {
        // Grayscale ramp 232-255
        let level = (i - 232) * 10 + 8;
        format!("#{:02x}{:02x}{:02x}", level, level, level)
    }
}

/// The 16 base ANSI colours (Linux console palette), as CSS hex strings.
static ANSI_COLORS: [&str; 16] = [
    "#000000", // 0  black
    "#aa0000", // 1  red
    "#00aa00", // 2  green
    "#aa5500", // 3  yellow (dark)
    "#0000aa", // 4  blue
    "#aa00aa", // 5  magenta
    "#00aaaa", // 6  cyan
    "#aaaaaa", // 7  white (light grey)
    "#555555", // 8  bright black (dark grey)
    "#ff5555", // 9  bright red
    "#55ff55", // 10 bright green
    "#ffff55", // 11 bright yellow
    "#5555ff", // 12 bright blue
    "#ff55ff", // 13 bright magenta
    "#55ffff", // 14 bright cyan
    "#ffffff", // 15 bright white
];

// ── Regex patterns ────────────────────────────────────────────────────────────

struct Patterns {
    xterm256: Regex,
    ansi: Regex,
    malformed: Regex,
}

static PATTERNS: OnceLock<Patterns> = OnceLock::new();

fn patterns() -> &'static Patterns {
    PATTERNS.get_or_init(|| Patterns {
        xterm256: Regex::new(r"\x1b\[(?:38|48);5;(\d+)m").unwrap(),
        ansi: Regex::new(r"\x1b\[([\d;]*)m").unwrap(),
        malformed: Regex::new(r"\x1b\[[\d;]*[A-Za-z]").unwrap(),
    })
}

// ── Tokenizer ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum Token<'a> {
    Text(&'a str),
    Backspace,
    Xterm256Fg(u8),
    Xterm256Bg(u8),
    Ansi(Vec<u32>),  // parsed code list from a CSI m sequence
    Ignore,
}

/// Tokenize `input` into a sequence of tokens.
fn tokenize(input: &str) -> Vec<Token<'_>> {
    let p = patterns();
    let mut tokens = Vec::new();
    let mut pos = 0;

    while pos < input.len() {
        let remaining = &input[pos..];

        // Find the next escape/control sequence
        let esc_pos = remaining
            .find('\x08')
            .map(|i| (i, 0usize))
            .into_iter()
            .chain(
                remaining
                    .find('\x1b')
                    .map(|i| (i, 1usize)),
            )
            .min_by_key(|(i, _)| *i);

        match esc_pos {
            None => {
                // No more control characters — rest is plain text
                if !remaining.is_empty() {
                    tokens.push(Token::Text(remaining));
                }
                break;
            }
            Some((0, _)) => {
                // Control char at current position
                if remaining.starts_with('\x08') {
                    tokens.push(Token::Backspace);
                    pos += 1;
                } else {
                    // Try xterm-256 first (more specific)
                    if let Some(m) = p.xterm256.find(remaining) {
                        if m.start() == 0 {
                            let caps = p.xterm256.captures(remaining).unwrap();
                            let idx: u8 = caps[1].parse().unwrap_or(0);
                            let is_bg = remaining.starts_with("\x1b[48");
                            if is_bg {
                                tokens.push(Token::Xterm256Bg(idx));
                            } else {
                                tokens.push(Token::Xterm256Fg(idx));
                            }
                            pos += m.end();
                            continue;
                        }
                    }
                    // Standard ANSI CSI m
                    if let Some(m) = p.ansi.find(remaining) {
                        if m.start() == 0 {
                            let caps = p.ansi.captures(remaining).unwrap();
                            let codes: Vec<u32> = caps[1]
                                .split(';')
                                .filter(|s| !s.is_empty())
                                .filter_map(|s| s.parse().ok())
                                .collect();
                            tokens.push(Token::Ansi(codes));
                            pos += m.end();
                            continue;
                        }
                    }
                    // Other CSI (cursor movement etc.) — ignore
                    if let Some(m) = p.malformed.find(remaining) {
                        if m.start() == 0 {
                            tokens.push(Token::Ignore);
                            pos += m.end();
                            continue;
                        }
                    }
                    // Bare ESC with nothing recognised — skip one byte
                    tokens.push(Token::Ignore);
                    pos += 1;
                }
            }
            Some((text_len, _)) => {
                // Plain text before the control char
                tokens.push(Token::Text(&remaining[..text_len]));
                pos += text_len;
            }
        }
    }

    tokens
}

// ── Style stack ───────────────────────────────────────────────────────────────

/// Marker for a single open `<span>` tag on the stack.
#[derive(Clone, Debug)]
struct Style;

/// Converts an ANSI code sequence and current tag stack into HTML output.
struct Renderer {
    stack: Vec<Style>,
    output: String,
}

impl Renderer {
    fn new() -> Self {
        Self {
            stack: Vec::new(),
            output: String::new(),
        }
    }

    fn push_style(&mut self, css: String) {
        self.output
            .push_str(&format!("<span style=\"{}\">", css));
        self.stack.push(Style);
    }

    fn reset_styles(&mut self) {
        for _ in self.stack.drain(..).rev() {
            self.output.push_str("</span>");
        }
    }

    fn apply_codes(&mut self, codes: &[u32]) {
        if codes.is_empty() {
            self.reset_styles();
            return;
        }
        for &code in codes {
            match code {
                0 => self.reset_styles(),
                1 => self.push_style("font-weight:bold".into()),
                2 => self.push_style("opacity:0.5".into()),
                3 => self.push_style("font-style:italic".into()),
                4 => self.push_style("text-decoration:underline".into()),
                5 | 6 => self.push_style("text-decoration:blink".into()),
                7 => self.push_style("filter:invert(100%)".into()),
                8 => self.push_style("visibility:hidden".into()),
                9 => self.push_style("text-decoration:line-through".into()),
                // Foreground colours: standard (30-37), bright (90-97)
                30..=37 => {
                    let color = ANSI_COLORS[(code - 30) as usize];
                    self.push_style(format!("color:{}", color));
                }
                38 => {} // handled as xterm-256 above
                39 => self.reset_styles(),
                // Background colours: standard (40-47), bright (100-107)
                40..=47 => {
                    let color = ANSI_COLORS[(code - 40) as usize];
                    self.push_style(format!("background-color:{}", color));
                }
                48 => {} // handled as xterm-256 above
                49 => self.reset_styles(),
                // Bright foreground (90-97)
                90..=97 => {
                    let color = ANSI_COLORS[(code - 90 + 8) as usize];
                    self.push_style(format!("color:{}", color));
                }
                // Bright background (100-107)
                100..=107 => {
                    let color = ANSI_COLORS[(code - 100 + 8) as usize];
                    self.push_style(format!("background-color:{}", color));
                }
                _ => {}
            }
        }
    }

    fn apply_xterm_fg(&mut self, idx: u8) {
        let color = xterm256_color(idx);
        self.push_style(format!("color:{}", color));
    }

    fn apply_xterm_bg(&mut self, idx: u8) {
        let color = xterm256_color(idx);
        self.push_style(format!("background-color:{}", color));
    }

    fn write_text(&mut self, text: &str) {
        self.output.push_str(&html_escape::encode_text(text));
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Convert a string containing ANSI escape sequences to an HTML fragment.
/// All spans are properly closed at the end.
pub fn ansi_to_html(input: &str) -> String {
    let tokens = tokenize(input);
    let mut r = Renderer::new();

    for token in tokens {
        match token {
            Token::Text(t) => r.write_text(t),
            Token::Backspace => {
                // Remove the last character from output (best-effort)
                if let Some(idx) = r.output.rfind(|c: char| c != '>') {
                    r.output.truncate(idx);
                }
            }
            Token::Xterm256Fg(idx) => r.apply_xterm_fg(idx),
            Token::Xterm256Bg(idx) => r.apply_xterm_bg(idx),
            Token::Ansi(codes) => r.apply_codes(&codes),
            Token::Ignore => {}
        }
    }

    // Close any still-open spans
    r.reset_styles();
    r.output
}

/// Streaming version: convert chunks incrementally.
/// Note: escape sequences must not be split across chunks.
pub fn ansi_chunk_to_html(input: &str) -> String {
    ansi_to_html(input)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passthrough() {
        assert_eq!(ansi_to_html("hello world"), "hello world");
    }

    #[test]
    fn html_entities_escaped() {
        assert_eq!(ansi_to_html("<b>&</b>"), "&lt;b&gt;&amp;&lt;/b&gt;");
    }

    #[test]
    fn bold() {
        let input = "\x1b[1mhello\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("font-weight:bold"));
        assert!(out.contains("hello"));
        assert!(out.ends_with("</span>"));
    }

    #[test]
    fn foreground_colour() {
        let input = "\x1b[31mred\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("color:#aa0000"));
        assert!(out.contains("red"));
    }

    #[test]
    fn xterm256_foreground() {
        // index 196 = bright red in the 6x6x6 cube
        let input = "\x1b[38;5;196mred\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("color:#"));
        assert!(out.contains("red"));
    }

    #[test]
    fn xterm256_grayscale() {
        let input = "\x1b[38;5;240mtext\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("color:#"));
    }

    #[test]
    fn unclosed_spans_are_closed() {
        let input = "\x1b[1mbold without reset";
        let out = ansi_to_html(input);
        assert!(out.ends_with("</span>"));
    }

    #[test]
    fn xterm256_palette_index0() {
        assert_eq!(xterm256_color(0), ANSI_COLORS[0]);
    }

    #[test]
    fn xterm256_palette_cube() {
        // index 16 = first entry of the 6x6x6 cube: r=0,g=0,b=0 → black
        assert_eq!(xterm256_color(16), "#000000");
        // index 231 = last of cube: r=5,g=5,b=5 → white
        assert_eq!(xterm256_color(231), "#ffffff");
    }

    #[test]
    fn xterm256_palette_grayscale() {
        // index 232: level = 0*10+8 = 8 → #080808
        assert_eq!(xterm256_color(232), "#080808");
        // index 255: level = 23*10+8 = 238 → #eeeeee
        assert_eq!(xterm256_color(255), "#eeeeee");
    }
}
