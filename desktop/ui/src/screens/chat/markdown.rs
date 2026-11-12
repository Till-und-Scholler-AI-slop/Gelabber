//! Message markdown, ported 1:1 from web/src/messages/markdown.ts: the
//! Discord-flavoured subset parsed into a small tree. Indices are chars
//! (code points) where the web counts UTF-16 units; both agree outside
//! astral characters, which no rule here looks at.

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Em(Vec<Inline>),
    Underline(Vec<Inline>),
    Strike(Vec<Inline>),
    Spoiler(Vec<Inline>),
    Code(String),
    Link {
        href: String,
        text: String,
        suppressed: bool,
    },
    /// A lower-cased user id.
    Mention(String),
    Everyone,
    Here,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Quote(Vec<Inline>),
    CodeBlock { lang: Option<String>, text: String },
}

const MAX_DEPTH: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Strong,
    Underline,
    Strike,
    Spoiler,
    EmStar,
    EmUnderscore,
}

impl Mark {
    fn text(self) -> &'static str {
        match self {
            Self::Strong => "**",
            Self::Underline => "__",
            Self::Strike => "~~",
            Self::Spoiler => "||",
            Self::EmStar => "*",
            Self::EmUnderscore => "_",
        }
    }

    fn node(self, children: Vec<Inline>) -> Inline {
        match self {
            Self::Strong => Inline::Strong(children),
            Self::Underline => Inline::Underline(children),
            Self::Strike => Inline::Strike(children),
            Self::Spoiler => Inline::Spoiler(children),
            Self::EmStar | Self::EmUnderscore => Inline::Em(children),
        }
    }
}

// Longer marks first, so `**` is bold before it is tried as two italics.
const MARKS: [Mark; 6] = [
    Mark::Strong,
    Mark::Underline,
    Mark::Strike,
    Mark::Spoiler,
    Mark::EmStar,
    Mark::EmUnderscore,
];

fn is_space(c: char) -> bool {
    c.is_whitespace()
}

fn is_alnum(c: char) -> bool {
    c.is_alphanumeric()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn starts_with(src: &[char], i: usize, pat: &str) -> bool {
    let n = pat.chars().count();
    src.len() >= i + n && src[i..i + n].iter().copied().eq(pat.chars())
}

fn starts_with_ci(src: &[char], i: usize, pat: &str) -> bool {
    let n = pat.chars().count();
    src.len() >= i + n
        && src[i..i + n]
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .eq(pat.chars())
}

fn find(src: &[char], from: usize, pat: &str) -> Option<usize> {
    (from..src.len()).find(|&i| starts_with(src, i, pat))
}

fn string(src: &[char]) -> String {
    src.iter().collect()
}

/// The message as blocks; an empty message has none.
pub fn parse_message(src: &str) -> Vec<Block> {
    let src: Vec<char> = src.chars().collect();
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < src.len() {
        if src[i] == '\\' && i + 1 < src.len() {
            text.push(src[i]);
            text.push(src[i + 1]);
            i += 2;
            continue;
        }
        if starts_with(&src, i, "```")
            && let Some(close) = find(&src, i + 3, "```")
            && let Some(code) = code_block(&src[i + 3..close])
        {
            let before = text.strip_suffix('\n').unwrap_or(&text).to_owned();
            push_text(&mut blocks, &before);
            blocks.push(code);
            text.clear();
            i = close + 3;
            if src.get(i) == Some(&'\n') {
                i += 1;
            }
            continue;
        }
        text.push(src[i]);
        i += 1;
    }
    push_text(&mut blocks, &text);
    blocks
}

fn code_block(body: &[char]) -> Option<Block> {
    // `^([A-Za-z0-9_+#.-]{1,32})\n`
    let lang_len = body
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || "_+#.-".contains(**c))
        .count();
    let lang = (1..=32).contains(&lang_len) && body.get(lang_len) == Some(&'\n');
    let mut text = if lang {
        string(&body[lang_len + 1..])
    } else {
        string(body)
    };
    if let Some(rest) = text.strip_prefix('\n') {
        text = rest.to_owned();
    }
    if let Some(rest) = text.strip_suffix('\n') {
        text = rest.to_owned();
    }
    if text.trim().is_empty() {
        return None;
    }
    Some(Block::CodeBlock {
        lang: lang.then(|| string(&body[..lang_len]).to_lowercase()),
        text,
    })
}

/// Plain text between code blocks: paragraphs and quotes, line by line.
fn push_text(blocks: &mut Vec<Block>, text: &str) {
    if text.is_empty() {
        return;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut plain: Vec<&str> = Vec::new();
    let mut quoted: Vec<&str> = Vec::new();
    fn flush_plain(blocks: &mut Vec<Block>, plain: &mut Vec<&str>) {
        if !plain.is_empty() {
            let joined = plain.join("\n");
            if !joined.is_empty() {
                blocks.push(Block::Paragraph(parse_inline(&joined)));
            }
        }
        plain.clear();
    }
    fn flush_quote(blocks: &mut Vec<Block>, quoted: &mut Vec<&str>) {
        if !quoted.is_empty() {
            blocks.push(Block::Quote(parse_inline(&quoted.join("\n"))));
        }
        quoted.clear();
    }
    for (n, line) in lines.iter().enumerate() {
        if let Some(rest) = line.strip_prefix(">>> ") {
            flush_plain(blocks, &mut plain);
            quoted.push(rest);
            quoted.extend(lines[n + 1..].iter().copied());
            break;
        }
        if let Some(rest) = line.strip_prefix("> ") {
            flush_plain(blocks, &mut plain);
            quoted.push(rest);
        } else {
            flush_quote(blocks, &mut quoted);
            plain.push(line);
        }
    }
    flush_plain(blocks, &mut plain);
    flush_quote(blocks, &mut quoted);
}

pub fn parse_inline(src: &str) -> Vec<Inline> {
    let chars: Vec<char> = src.chars().collect();
    inline(&chars, 0)
}

fn inline(src: &[char], depth: usize) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut text = String::new();
    let push = |out: &mut Vec<Inline>, text: &mut String, node: Inline| {
        if !text.is_empty() {
            out.push(Inline::Text(std::mem::take(text)));
        }
        out.push(node);
    };
    let mut unclosed: Vec<Mark> = Vec::new();
    let mut i = 0;
    'outer: while i < src.len() {
        let c = src[i];
        if c == '\\' && i + 1 < src.len() && !is_alnum(src[i + 1]) && !is_space(src[i + 1]) {
            text.push(src[i + 1]);
            i += 2;
            continue;
        }
        if c == '`' {
            let run = run_length(src, i, '`');
            match code_span_end(src, i, run) {
                None => {
                    text.extend(&src[i..i + run]);
                    i += run;
                }
                Some(end) => {
                    let mut code = string(&src[i + run..end - run]);
                    if code.chars().count() > 2
                        && code.starts_with(' ')
                        && code.ends_with(' ')
                        && !code.trim().is_empty()
                    {
                        code = code[1..code.len() - 1].to_owned();
                    }
                    push(&mut out, &mut text, Inline::Code(code));
                    i = end;
                }
            }
            continue;
        }
        if c == '<' {
            if let Some((id, len)) = mention_at(src, i) {
                push(&mut out, &mut text, Inline::Mention(id));
                i += len;
                continue;
            }
            if let Some((raw, len)) = angle_link_at(src, i)
                && let Some(href) = safe_href(&raw)
            {
                push(
                    &mut out,
                    &mut text,
                    Inline::Link {
                        href,
                        text: raw,
                        suppressed: true,
                    },
                );
                i += len;
                continue;
            }
        }
        if c == '@' && (i == 0 || !keyword_before(src[i - 1])) {
            for (word, node) in [("everyone", Inline::Everyone), ("here", Inline::Here)] {
                if starts_with(src, i + 1, word) {
                    let end = i + 1 + word.chars().count();
                    let blocked = src
                        .get(end)
                        .is_some_and(|n| n.is_alphanumeric() || "_@-".contains(*n));
                    if !blocked {
                        push(&mut out, &mut text, node);
                        i = end;
                        continue 'outer;
                    }
                }
            }
        }
        if let Some((raw, href)) = bare_url_at(src, i) {
            let len = raw.chars().count();
            push(
                &mut out,
                &mut text,
                Inline::Link {
                    href,
                    text: raw,
                    suppressed: false,
                },
            );
            i += len;
            continue;
        }
        if depth < MAX_DEPTH {
            for mark in MARKS {
                if unclosed.contains(&mark) {
                    continue;
                }
                if !starts_with(src, i, mark.text()) || !opens(src, i, mark) {
                    continue;
                }
                let from = i + mark.text().len();
                let close = find_closer(src, from, mark);
                if close.is_none() {
                    unclosed.push(mark);
                }
                let Some(close) = close.filter(|&c| c > from) else {
                    continue;
                };
                let children = inline(&src[from..close], depth + 1);
                push(&mut out, &mut text, mark.node(children));
                i = close + mark.text().len();
                continue 'outer;
            }
        }
        text.push(c);
        i += 1;
    }
    if !text.is_empty() {
        out.push(Inline::Text(text));
    }
    out
}

fn keyword_before(c: char) -> bool {
    c.is_alphanumeric() || "_@/-".contains(c)
}

fn opens(src: &[char], i: usize, mark: Mark) -> bool {
    let Some(&next) = src.get(i + mark.text().len()) else {
        return false;
    };
    match mark {
        Mark::EmStar => !is_space(next),
        Mark::EmUnderscore => !is_space(next) && (i == 0 || !is_word(src[i - 1])),
        _ => true,
    }
}

/// Where `mark` closes, scanning past escapes, code, links and mentions.
fn find_closer(src: &[char], from: usize, mark: Mark) -> Option<usize> {
    let text = mark.text();
    let single = text.len() == 1;
    let first = text.chars().next().unwrap_or('*');
    let mut j = from;
    while j < src.len() {
        let c = src[j];
        if c == '\\' {
            j += 2;
            continue;
        }
        if c == '`' {
            let run = run_length(src, j, '`');
            j = code_span_end(src, j, run).unwrap_or(j + run);
            continue;
        }
        if c == '<' {
            if let Some((_, len)) = mention_at(src, j) {
                j += len;
                continue;
            }
            if let Some((_, len)) = angle_link_at(src, j) {
                j += len;
                continue;
            }
        }
        if let Some((raw, _)) = bare_url_at(src, j) {
            j += raw.chars().count();
            continue;
        }
        if starts_with(src, j, text) {
            if single {
                // A double mark inside an italic is a nested bold or underline.
                if src.get(j + 1) == Some(&first) {
                    j += run_length(src, j, first);
                    continue;
                }
                if mark == Mark::EmStar && j > 0 && is_space(src[j - 1]) {
                    j += 1;
                    continue;
                }
                if mark == Mark::EmUnderscore && src.get(j + 1).is_some_and(|n| is_word(*n)) {
                    j += 1;
                    continue;
                }
                return Some(j);
            }
            if matches!(mark, Mark::Strong | Mark::Underline) {
                // `***a***`: the outer bold closes on the run's last two marks.
                return Some(j + run_length(src, j, first) - 2);
            }
            return Some(j);
        }
        j += 1;
    }
    None
}

fn run_length(src: &[char], i: usize, ch: char) -> usize {
    src[i..].iter().take_while(|c| **c == ch).count()
}

/// The end of a code span opened by `run` backticks at `i`.
fn code_span_end(src: &[char], i: usize, run: usize) -> Option<usize> {
    let mut j = i + run;
    while j < src.len() {
        let next = (j..src.len()).find(|&k| src[k] == '`')?;
        let len = run_length(src, next, '`');
        if len == run {
            return Some(next + len);
        }
        j = next + len;
    }
    None
}

/// `<@uuid>`: the lower-cased id and the token length.
fn mention_at(src: &[char], i: usize) -> Option<(String, usize)> {
    if !starts_with(src, i, "<@") {
        return None;
    }
    let groups = [8, 4, 4, 4, 12];
    let mut j = i + 2;
    let mut id = String::new();
    for (n, len) in groups.iter().enumerate() {
        for _ in 0..*len {
            let c = *src.get(j)?;
            if !c.is_ascii_hexdigit() {
                return None;
            }
            id.push(c.to_ascii_lowercase());
            j += 1;
        }
        if n < 4 {
            if src.get(j) != Some(&'-') {
                return None;
            }
            id.push('-');
            j += 1;
        }
    }
    (src.get(j) == Some(&'>')).then_some((id, j + 1 - i))
}

fn url_char(c: char) -> bool {
    !c.is_whitespace() && !"<>\"".contains(c)
}

/// `<http(s)://…>` or `<mailto:…>`: the inner text and the token length.
fn angle_link_at(src: &[char], i: usize) -> Option<(String, usize)> {
    if src.get(i) != Some(&'<') {
        return None;
    }
    let start = i + 1;
    let scheme = ["https://", "http://", "mailto:"]
        .iter()
        .find(|s| starts_with_ci(src, start, s))?;
    let mut j = start + scheme.len();
    let body = j;
    while j < src.len() && url_char(src[j]) {
        j += 1;
    }
    if j == body || src.get(j) != Some(&'>') {
        return None;
    }
    Some((string(&src[start..j]), j + 1 - i))
}

/// A bare http(s) URL at `i`, trailing punctuation left out: raw text, href.
fn bare_url_at(src: &[char], i: usize) -> Option<(String, String)> {
    let c = *src.get(i)?;
    if c != 'h' && c != 'H' {
        return None;
    }
    if i > 0 && is_alnum(src[i - 1]) {
        return None;
    }
    let scheme = ["https://", "http://"]
        .iter()
        .find(|s| starts_with_ci(src, i, s))?;
    let mut j = i + scheme.len();
    let body = j;
    while j < src.len() && j - i < 2048 && url_char(src[j]) {
        j += 1;
    }
    if j == body {
        return None;
    }
    let text = trim_url(&src[i..j]);
    let href = safe_href(&text)?;
    Some((text, href))
}

fn trim_url(url: &[char]) -> String {
    let count = |ch: char| url.iter().filter(|c| **c == ch).count() as i64;
    let mut paren = count(')') - count('(');
    let mut bracket = count(']') - count('[');
    let mut end = url.len();
    while end > 0 {
        let c = url[end - 1];
        if ".,:;!?'\"*_~|".contains(c) {
            end -= 1;
            continue;
        }
        if c == ')' && paren > 0 {
            paren -= 1;
            end -= 1;
            continue;
        }
        if c == ']' && bracket > 0 {
            bracket -= 1;
            end -= 1;
            continue;
        }
        break;
    }
    string(&url[..end])
}

/// The link target when it is http(s) with a host, or mailto with a path.
pub fn safe_href(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    match url.scheme() {
        "http" | "https" => url
            .host_str()
            .filter(|h| !h.is_empty())
            .map(|_| url.as_str().to_owned()),
        "mailto" => (!url.path().is_empty()).then(|| url.as_str().to_owned()),
        _ => None,
    }
}

/// `plainText`: the message as one line for a reply line, a preview or a
/// notification. The formatting marks go, a spoiler stays hidden as `▒▒▒`
/// and mentions stay tokens for `readable_mentions`.
pub fn plain_text(src: &str) -> String {
    fn inline(nodes: &[Inline], out: &mut String) {
        for node in nodes {
            match node {
                Inline::Text(t) | Inline::Code(t) | Inline::Link { text: t, .. } => {
                    out.push_str(t);
                }
                Inline::Mention(id) => {
                    out.push_str("<@");
                    out.push_str(id);
                    out.push('>');
                }
                Inline::Everyone => out.push_str("@everyone"),
                Inline::Here => out.push_str("@here"),
                Inline::Spoiler(_) => out.push_str("▒▒▒"),
                Inline::Strong(c) | Inline::Em(c) | Inline::Underline(c) | Inline::Strike(c) => {
                    inline(c, out);
                }
            }
        }
    }
    let mut out = String::with_capacity(src.len());
    for block in parse_message(src) {
        out.push(' ');
        match block {
            Block::Paragraph(nodes) | Block::Quote(nodes) => inline(&nodes, &mut out),
            Block::CodeBlock { text, .. } => out.push_str(&text),
        }
    }
    let words = out.split_whitespace();
    // The server cuts a reply's excerpt with "…", maybe inside a mark that
    // then stays unclosed ("so **halb…"): what is left of it goes too.
    if src.ends_with('…') {
        words.map(drop_cut_marks).collect::<Vec<_>>().join(" ")
    } else {
        words.collect::<Vec<_>>().join(" ")
    }
}

/// `dropCutMarks`: a word of a cut excerpt without an unclosed mark run at
/// its start, or a half closing one before the "…".
fn drop_cut_marks(word: &str) -> String {
    let mark = |c: char| matches!(c, '*' | '_' | '~');
    let rest = word.trim_start_matches(mark);
    let word = if (1..=3).contains(&(word.len() - rest.len())) && !rest.is_empty() {
        rest
    } else {
        word
    };
    match word.strip_suffix('…') {
        Some(body) => format!("{}…", body.trim_end_matches(mark)), // i18n-ok
        None => word.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_drops_the_marks() {
        assert_eq!(
            plain_text("Ich bin dabei. **Film oder Spiel?**"),
            "Ich bin dabei. Film oder Spiel?"
        );
        assert_eq!(plain_text("*a* __b__ ~~c~~ `d` \\*e\\*"), "a b c d *e*");
        assert_eq!(
            plain_text("> zitiert\nund ```rs\nfn x() {}\n``` danach"),
            "zitiert und fn x() {} danach"
        );
        assert_eq!(plain_text("sag ||Snape stirbt|| nicht"), "sag ▒▒▒ nicht");
        let id = "0b9e4a9c-58e2-4f4e-9d65-9b3b0b8c7d11";
        assert_eq!(
            plain_text(&format!("hi <@{id}> @here <https://a.b/>")),
            format!("hi <@{id}> @here https://a.b/")
        );
    }

    #[test]
    fn plain_text_drops_the_marks_a_cut_leaves_unclosed() {
        assert_eq!(plain_text("so **halb…"), "so halb…");
        assert_eq!(plain_text("so **halb fertig*…"), "so halb fertig…");
        assert_eq!(plain_text("~~weg__ und _da…"), "weg__ und da…");
        // Only a cut excerpt: a whole message keeps what it typed.
        assert_eq!(plain_text("so **halb"), "so **halb");
        assert_eq!(plain_text("a * b…"), "a * b…");
    }

    fn text(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    #[test]
    fn inline_marks() {
        assert_eq!(
            parse_inline("Ich bin dabei. **Film oder Spiel?**"),
            vec![
                text("Ich bin dabei. "),
                Inline::Strong(vec![text("Film oder Spiel?")])
            ]
        );
        assert_eq!(
            parse_inline("auf *Brettspiele* oder"),
            vec![
                text("auf "),
                Inline::Em(vec![text("Brettspiele")]),
                text(" oder")
            ]
        );
        assert_eq!(
            parse_inline("snake_case_name"),
            vec![text("snake_case_name")]
        );
        assert_eq!(
            parse_inline("***a***"),
            vec![Inline::Strong(vec![Inline::Em(vec![text("a")])])]
        );
        assert_eq!(
            parse_inline("um `20:00` im"),
            vec![text("um "), Inline::Code("20:00".into()), text(" im")]
        );
        assert_eq!(parse_inline("\\*x\\*"), vec![text("*x*")]);
        assert_eq!(
            parse_inline("||geheim||"),
            vec![Inline::Spoiler(vec![text("geheim")])]
        );
    }

    #[test]
    fn links_and_mentions() {
        assert_eq!(
            parse_inline("see https://example.org/a)."),
            vec![
                text("see "),
                Inline::Link {
                    href: "https://example.org/a".into(),
                    text: "https://example.org/a".into(),
                    suppressed: false
                },
                text(").")
            ]
        );
        assert_eq!(
            parse_inline("<https://x.org>"),
            vec![Inline::Link {
                href: "https://x.org/".into(),
                text: "https://x.org".into(),
                suppressed: true
            }]
        );
        let id = "3796349f-6f8f-4bcf-826f-d2237e2fe9d3";
        assert_eq!(
            parse_inline(&format!("hi <@{}>!", id.to_uppercase())),
            vec![text("hi "), Inline::Mention(id.into()), text("!")]
        );
        assert_eq!(
            parse_inline("@everyone mail@here"),
            vec![Inline::Everyone, text(" mail@here")]
        );
        assert_eq!(
            parse_inline("javascript:alert(1)"),
            vec![text("javascript:alert(1)")]
        );
    }

    #[test]
    fn blocks() {
        let blocks = parse_message("Skript:\n```rust\nfn a() {}\n```\nnach");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph(vec![text("Skript:")]),
                Block::CodeBlock {
                    lang: Some("rust".into()),
                    text: "fn a() {}".into()
                },
                Block::Paragraph(vec![text("nach")]),
            ]
        );
        let quote = parse_message("Regeln:\n> eins\n> zwei\nweiter");
        assert_eq!(
            quote,
            vec![
                Block::Paragraph(vec![text("Regeln:")]),
                Block::Quote(vec![text("eins\nzwei")]),
                Block::Paragraph(vec![text("weiter")]),
            ]
        );
        assert_eq!(
            parse_message(">>> alles\nhier"),
            vec![Block::Quote(vec![text("alles\nhier")])]
        );
        assert_eq!(
            // Blank body: no code block, an inline code span instead.
            parse_message("```   ```"),
            vec![Block::Paragraph(vec![Inline::Code("   ".into())])]
        );
        assert!(parse_message("").is_empty());
    }
}
