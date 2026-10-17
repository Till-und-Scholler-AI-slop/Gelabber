//! Which links of a message get a preview: the server's reading of the web
//! client's `previewUrls` (`web/src/messages/markdown.ts`). Bare `http(s)`
//! links count; links in code, in `||spoilers||` and in `<url>` brackets
//! (Discord's "no preview") do not. At most [`MAX_PER_MESSAGE`], in order,
//! without duplicates.

use url::Url;

pub const MAX_PER_MESSAGE: usize = 3;

pub fn preview_urls(content: &str) -> Vec<Url> {
    let chars: Vec<char> = content.chars().collect();
    let mut found: Vec<Url> = Vec::new();
    let mut i = 0;
    while i < chars.len() && found.len() < MAX_PER_MESSAGE {
        let c = chars[i];
        if c == '\\'
            && chars
                .get(i + 1)
                .is_some_and(|n| !n.is_alphanumeric() && !n.is_whitespace())
        {
            i += 2;
            continue;
        }
        if starts_with(&chars, i, "```")
            && let Some(end) = find(&chars, i + 3, "```")
        {
            i = end + 3;
            continue;
        }
        if c == '`' {
            let run = run_length(&chars, i, '`');
            i = code_span_end(&chars, i, run).unwrap_or(i + run);
            continue;
        }
        if starts_with(&chars, i, "||")
            && let Some(end) = find(&chars, i + 2, "||").filter(|end| *end > i + 2)
        {
            i = end + 2;
            continue;
        }
        if c == '<'
            && let Some(len) = angle_link(&chars, i)
        {
            i += len;
            continue;
        }
        if let Some((url, len)) = bare_url(&chars, i) {
            if !found.contains(&url) {
                found.push(url);
            }
            i += len;
            continue;
        }
        i += 1;
    }
    found
}

fn starts_with(chars: &[char], i: usize, pat: &str) -> bool {
    (i..)
        .zip(pat.chars())
        .all(|(j, p)| chars.get(j) == Some(&p))
}

fn find(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    (from..chars.len()).find(|&j| starts_with(chars, j, pat))
}

fn run_length(chars: &[char], i: usize, ch: char) -> usize {
    chars[i..].iter().take_while(|c| **c == ch).count()
}

fn code_span_end(chars: &[char], i: usize, run: usize) -> Option<usize> {
    let mut j = i + run;
    while j < chars.len() {
        if chars[j] == '`' {
            let len = run_length(chars, j, '`');
            if len == run {
                return Some(j + len);
            }
            j += len;
        } else {
            j += 1;
        }
    }
    None
}

/// `<https://…>`, `<http://…>` or `<mailto:…>`: its length in chars.
fn angle_link(chars: &[char], i: usize) -> Option<usize> {
    let rest: String = chars[i + 1..].iter().take(2048).collect();
    let lower = rest.to_ascii_lowercase();
    if !["https://", "http://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return None;
    }
    let body = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '<' | '>' | '"'))
        .count();
    (rest.chars().nth(body) == Some('>')).then_some(body + 2)
}

/// A bare link at `i`: not glued to a preceding letter or digit, trailing
/// punctuation and unbalanced closing brackets left out.
fn bare_url(chars: &[char], i: usize) -> Option<(Url, usize)> {
    if !matches!(chars[i], 'h' | 'H') || (i > 0 && chars[i - 1].is_alphanumeric()) {
        return None;
    }
    let head: String = chars[i..]
        .iter()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    if !head.starts_with("http://") && !head.starts_with("https://") {
        return None;
    }
    let mut end = i;
    while end < chars.len()
        && end - i < 2048
        && !chars[end].is_whitespace()
        && !matches!(chars[end], '<' | '>' | '"')
    {
        end += 1;
    }
    loop {
        let &c = chars.get(end.wrapping_sub(1)).filter(|_| end > i)?;
        if ".,:;!?'\"*_~|".contains(c) {
            end -= 1;
            continue;
        }
        let open = match c {
            ')' => Some('('),
            ']' => Some('['),
            _ => None,
        };
        if let Some(open) = open {
            let span = &chars[i..end];
            let closes = span.iter().filter(|x| **x == c).count();
            let opens = span.iter().filter(|x| **x == open).count();
            if closes > opens {
                end -= 1;
                continue;
            }
        }
        break;
    }
    let text: String = chars[i..end].iter().collect();
    let url = Url::parse(&text).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    Some((url, end - i))
}

#[cfg(test)]
mod tests {
    use super::preview_urls;

    fn urls(content: &str) -> Vec<String> {
        preview_urls(content)
            .into_iter()
            .map(String::from)
            .collect()
    }

    #[test]
    fn takes_bare_links_in_order_without_duplicates() {
        assert_eq!(
            urls(
                "a https://a.example/x, (https://en.wikipedia.org/wiki/Foo_(bar)) https://a.example/x"
            ),
            [
                "https://a.example/x",
                "https://en.wikipedia.org/wiki/Foo_(bar)"
            ]
        );
        assert_eq!(urls("**https://b.example/**"), ["https://b.example/"]);
    }

    #[test]
    fn stops_at_three() {
        assert_eq!(
            urls("https://a.example https://b.example https://c.example https://d.example").len(),
            3
        );
    }

    #[test]
    fn skips_code_spoilers_and_suppressed_links() {
        assert!(
            urls("`https://a.example` ```\nhttps://b.example\n``` ||https://c.example|| <https://d.example>")
                .is_empty()
        );
        assert!(urls("xhttps://a.example \\https://b.example").len() == 1);
    }

    #[test]
    fn ignores_other_schemes() {
        assert!(urls("ftp://a.example javascript:alert(1) mailto:a@b.example").is_empty());
        assert!(urls("https:// http://").is_empty());
    }

    /// The cases of `previewUrls` in `web/src/messages/markdown.test.ts`.
    #[test]
    fn agrees_with_the_web_client() {
        assert_eq!(
            urls(
                "https://a.example <https://b.example> ||https://c.example|| `https://d.example` https://a.example https://e.example https://f.example https://g.example\n```\nhttps://h.example\n```"
            ),
            [
                "https://a.example/",
                "https://e.example/",
                "https://f.example/"
            ]
        );
    }
}
