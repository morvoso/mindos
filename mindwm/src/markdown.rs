//! Just enough Markdown for the Mind bar.
//!
//! The model answers in Markdown — `**bold**`, `` `code` ``, headings,
//! bullets, fenced blocks, links — and the bar used to draw the markers as
//! text. This turns an answer into styled [`Span`]s the text renderer can lay
//! out in one pass: the markers disappear and what they meant shows up as a
//! heavier face, the mono face or a plain "text (url)".
//!
//! It is deliberately a line-at-a-time reader, not a parser: no nesting, no
//! tables, no reference links. Anything it does not recognise is left as it
//! was written, which is the right answer for a chat bar.

/// How a run of text is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Normal,
    /// `**bold**`, `__bold__` and headings.
    Strong,
    /// `*italic*` and `_italic_`; there is no italic face, so it is drawn a
    /// touch heavier than body text.
    Emphasis,
    /// `` `code` `` and fenced blocks.
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// The markers a bullet can start with.
const BULLETS: [&str; 3] = ["- ", "* ", "+ "];

/// Split Markdown into styled spans. Newlines are kept inside the spans, so
/// the result lays out as one block of text.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut fenced = false;
    for (i, raw) in text.split('\n').enumerate() {
        if i > 0 {
            push(&mut out, "\n", Style::Normal);
        }
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            push(&mut out, line, Style::Code);
            continue;
        }
        // A heading is the whole line in the heavy face, without its hashes.
        if let Some(rest) = heading(trimmed) {
            inline(rest, Style::Strong, &mut out);
            continue;
        }
        // A rule is a line of its own; the bar has no room for one.
        if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_') {
            continue;
        }
        let indent = &line[..line.len() - trimmed.len()];
        let quoted = trimmed.strip_prefix("> ").or_else(|| trimmed.strip_prefix(">"));
        let body = quoted.unwrap_or(trimmed);
        if let Some(rest) = BULLETS.iter().find_map(|b| body.strip_prefix(b)) {
            push(&mut out, &format!("{indent}• "), Style::Normal);
            inline(rest, Style::Normal, &mut out);
            continue;
        }
        push(&mut out, indent, Style::Normal);
        inline(body, Style::Normal, &mut out);
    }
    out
}

/// `### Title` → `Title`.
fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        line[hashes..].strip_prefix(' ').map(str::trim)
    } else {
        None
    }
}

/// Append `text`, merging it into the last span when the style matches.
fn push(out: &mut Vec<Span>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(last) if last.style == style => last.text.push_str(text),
        _ => out.push(Span { text: text.into(), style }),
    }
}

/// The inline markers inside one line. `base` is what unmarked text gets
/// (headings pass `Strong`, so emphasis inside a heading stays heavy).
fn inline(line: &str, base: Style, out: &mut Vec<Span>) {
    let b = line.as_bytes();
    let mut plain = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        // A backslash escapes the next character, marker or not.
        if c == b'\\' && i + 1 < b.len() {
            let ch = line[i + 1..].chars().next().unwrap();
            plain.push(ch);
            i += 1 + ch.len_utf8();
            continue;
        }
        let marker: Option<(&str, Style)> = match c {
            b'`' => Some(("`", Style::Code)),
            b'*' | b'_' if b.get(i + 1) == Some(&c) => {
                Some((if c == b'*' { "**" } else { "__" }, Style::Strong))
            }
            b'*' | b'_' => Some((if c == b'*' { "*" } else { "_" }, Style::Emphasis)),
            _ => None,
        };
        if let Some((mark, style)) = marker {
            if let Some((inner, next)) = closed(line, i, mark, style != Style::Code) {
                push(out, &plain, base);
                plain.clear();
                // Code is code even inside a heading; the rest follows `base`.
                push(out, inner, if style == Style::Code { style } else { heavier(base, style) });
                i = next;
                continue;
            }
        }
        if c == b'[' {
            if let Some((label, url, next)) = link(line, i) {
                push(out, &plain, base);
                plain.clear();
                inline(label, base, out);
                if !url.is_empty() && url != label {
                    push(out, &format!(" ({url})"), base);
                }
                i = next;
                continue;
            }
        }
        let ch = line[i..].chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }
    push(out, &plain, base);
}

/// Emphasis inside a heading must not make it lighter.
fn heavier(base: Style, style: Style) -> Style {
    if base == Style::Strong {
        Style::Strong
    } else {
        style
    }
}

/// The text between the marker at `start` and its closing partner, plus the
/// index just after it. `None` when the marker is never closed on this line —
/// a lone `*` is then drawn as itself.
///
/// With `flanking` the emphasis rules apply: the marked text may not begin or
/// end with a space (so `2 * 3 * 4` is arithmetic, not emphasis) and an `_`
/// may not sit inside a word (so `nvngx_dlss_swap` keeps its underscores).
fn closed<'a>(line: &'a str, start: usize, mark: &str, flanking: bool) -> Option<(&'a str, usize)> {
    let from = start + mark.len();
    let end = line[from..].find(mark)? + from;
    let inner = &line[from..end];
    if inner.is_empty() {
        return None;
    }
    let after = end + mark.len();
    if flanking {
        if inner.starts_with(char::is_whitespace) || inner.ends_with(char::is_whitespace) {
            return None;
        }
        if mark.starts_with('_') {
            let before_ok = line[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
            let after_ok = line[after..].chars().next().is_none_or(|c| !c.is_alphanumeric());
            if !before_ok || !after_ok {
                return None;
            }
        }
    }
    Some((inner, after))
}

/// `[label](url)` at `start` → the label, the URL and the index after it.
fn link(line: &str, start: usize) -> Option<(&str, &str, usize)> {
    let close = line[start + 1..].find("](")? + start + 1;
    let end = line[close + 2..].find(')')? + close + 2;
    let url = line[close + 2..end].split_whitespace().next().unwrap_or("");
    Some((&line[start + 1..close], url, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Vec<(String, Style)> {
        spans(text).into_iter().map(|s| (s.text, s.style)).collect()
    }

    #[test]
    fn inline_markers_become_styles() {
        assert_eq!(
            s("Steam is **gold** on `protondb`."),
            vec![
                ("Steam is ".into(), Style::Normal),
                ("gold".into(), Style::Strong),
                (" on ".into(), Style::Normal),
                ("protondb".into(), Style::Code),
                (".".into(), Style::Normal),
            ]
        );
        assert_eq!(s("_quietly_"), vec![("quietly".into(), Style::Emphasis)]);
    }

    #[test]
    fn unclosed_and_escaped_markers_stay_literal() {
        assert_eq!(s("2 * 3 * 4 = 24"), vec![("2 * 3 * 4 = 24".into(), Style::Normal)]);
        assert_eq!(s("a \\*star\\*"), vec![("a *star*".into(), Style::Normal)]);
        assert_eq!(s("nvngx_dlss_swap"), vec![("nvngx_dlss_swap".into(), Style::Normal)]);
        assert_eq!(s("**"), vec![("**".into(), Style::Normal)]);
    }

    #[test]
    fn blocks_lose_their_markers() {
        assert_eq!(s("## Updates"), vec![("Updates".into(), Style::Strong)]);
        assert_eq!(
            s("- one\n- two"),
            vec![("• one\n• two".into(), Style::Normal)]
        );
        assert_eq!(
            s("```sh\npacman -Syu\n```"),
            vec![("\n".into(), Style::Normal), ("pacman -Syu".into(), Style::Code), ("\n".into(), Style::Normal)]
        );
        assert_eq!(s("> quoted"), vec![("quoted".into(), Style::Normal)]);
        assert_eq!(s("---"), vec![]);
    }

    #[test]
    fn links_keep_the_address() {
        assert_eq!(
            s("see [the wiki](https://wiki.archlinux.org/Foo) first"),
            vec![("see the wiki (https://wiki.archlinux.org/Foo) first".into(), Style::Normal)]
        );
        assert_eq!(s("[x](x)"), vec![("x".into(), Style::Normal)]);
    }

    #[test]
    fn a_heading_stays_heavy_throughout() {
        assert_eq!(s("# The *Mind*"), vec![("The Mind".into(), Style::Strong)]);
    }
}
