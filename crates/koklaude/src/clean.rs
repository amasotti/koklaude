//! Markdown → speakable prose.
//!
//! Claude's replies are markdown; read verbatim they're noise (code blocks,
//! `*`, `#`, link URLs). `clean` walks the parsed markdown and keeps only what's
//! worth hearing: prose, headings, list/table cell text, inline-code tokens.
//! Block elements become separate lines so espeak pauses between them.

// Unused until the hook (Phase 4) / transcript path feeds it — remove then.
#![allow(dead_code)]

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Strip markdown to plain, speakable text.
pub fn clean(markdown: &str) -> String {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;

    let mut out = String::new();
    let mut in_code_block = false;
    let mut in_image = false;

    for event in Parser::new_ext(markdown, opts) {
        match event {
            // Fenced/indented code: drop wholesale — code read aloud is noise.
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                out.push('\n');
            }

            // Images: skip their alt text (rarely useful spoken).
            Event::Start(Tag::Image { .. }) => in_image = true,
            Event::End(TagEnd::Image) => in_image = false,

            // Prose. Inline `code` is kept (usually a short identifier/word).
            Event::Text(t) if !in_code_block && !in_image => out.push_str(&t),
            Event::Code(t) => out.push_str(&t),

            Event::SoftBreak | Event::HardBreak => out.push(' '),

            // Block boundaries → newline → espeak pause.
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::BlockQuote(_)
                | TagEnd::TableRow
                | TagEnd::TableHead,
            ) => out.push('\n'),

            // Separate table cells within a row.
            Event::End(TagEnd::TableCell) => out.push(' '),

            // HTML, rules, link wrappers, etc. — ignore (link *text* arrives as Text).
            _ => {}
        }
    }

    normalize(&out)
}

/// Collapse intra-line whitespace, drop blank lines, one block per line.
fn normalize(s: &str) -> String {
    s.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_unchanged() {
        assert_eq!(clean("Hello there."), "Hello there.");
    }

    #[test]
    fn strips_emphasis_and_headings() {
        assert_eq!(clean("# Title\n\nSome **bold** and *italic*."), "Title\nSome bold and italic.");
    }

    #[test]
    fn drops_fenced_code_block() {
        let md = "Before.\n\n```rust\nlet x = 5;\nfn main() {}\n```\n\nAfter.";
        assert_eq!(clean(md), "Before.\nAfter.");
    }

    #[test]
    fn keeps_inline_code_token() {
        assert_eq!(clean("Run `cargo test` now."), "Run cargo test now.");
    }

    #[test]
    fn link_keeps_text_drops_url() {
        assert_eq!(clean("See [the docs](https://example.com/x)."), "See the docs.");
    }

    #[test]
    fn list_items_become_lines() {
        assert_eq!(clean("- one\n- two\n- three"), "one\ntwo\nthree");
    }

    #[test]
    fn image_alt_dropped() {
        assert_eq!(clean("Look ![a big diagram](x.png) here."), "Look here.");
    }

    #[test]
    fn table_separator_row_gone() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        // header row + data row, no `|---|` noise
        assert_eq!(clean(md), "A B\n1 2");
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(clean("a    b\t\tc"), "a b c");
    }

    #[test]
    fn empty_input_empty_output() {
        assert_eq!(clean(""), "");
    }
}
