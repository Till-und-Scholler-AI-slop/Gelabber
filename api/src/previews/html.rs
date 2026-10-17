//! What a page says about itself: OpenGraph, Twitter card and `<title>`,
//! read from the head with a small tag scanner. No DOM, no scripts; the
//! result is plain text the client renders as text.

use url::Url;

pub const TITLE_MAX: usize = 256;
pub const DESCRIPTION_MAX: usize = 350;
pub const SITE_NAME_MAX: usize = 100;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Meta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    /// Absolute http(s) URL of the preview image.
    pub image: Option<Url>,
}

impl Meta {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.description.is_none() && self.image.is_none()
    }
}

/// The page's text: UTF-8 unless the header or a `<meta charset>` says
/// Latin-1 / Windows-1252 (read as Latin-1, close enough for a title).
pub fn decode(body: &[u8], charset: Option<&str>) -> String {
    let latin = |c: &str| {
        matches!(
            c,
            "iso-8859-1" | "latin1" | "windows-1252" | "us-ascii" | "ascii"
        )
    };
    let declared = charset.map(str::to_owned).or_else(|| meta_charset(body));
    if declared.as_deref().is_some_and(latin) {
        body.iter().map(|b| char::from(*b)).collect()
    } else {
        String::from_utf8_lossy(body).into_owned()
    }
}

fn meta_charset(body: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(&body[..body.len().min(4096)]).to_ascii_lowercase();
    let at = head.find("charset=")? + "charset=".len();
    let value: String = head[at..]
        .trim_start_matches(['"', '\''])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!value.is_empty()).then_some(value)
}

pub fn parse(html: &str, base: &Url) -> Meta {
    let mut og_title = None;
    let mut tw_title = None;
    let mut title_tag = None;
    let mut og_description = None;
    let mut tw_description = None;
    let mut description = None;
    let mut site_name = None;
    let mut og_image = None;
    let mut tw_image = None;

    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    while let Some(off) = lower[i..].find('<') {
        let at = i + off;
        let rest = &lower[at..];
        if rest.starts_with("<!--") {
            i = lower[at..]
                .find("-->")
                .map_or(lower.len(), |end| at + end + 3);
            continue;
        }
        if rest.starts_with("</head") || rest.starts_with("<body") {
            break;
        }
        let name: String = rest[1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let Some(close) = tag_end(&lower, at) else {
            break;
        };
        match name.as_str() {
            "script" | "style" | "noscript" | "template" => {
                let end = format!("</{name}");
                i = lower[close..].find(&end).map_or(lower.len(), |e| close + e);
                continue;
            }
            "title" if title_tag.is_none() => {
                let end = lower[close..]
                    .find("</title")
                    .map_or(lower.len(), |e| close + e);
                title_tag = Some(html[close..end].to_owned());
                i = end;
                continue;
            }
            "meta" => {
                let attrs = attributes(&html[at + 5..close - 1]);
                let get = |key: &str| {
                    attrs
                        .iter()
                        .find(|(k, _)| k == key)
                        .map(|(_, v)| v.as_str())
                };
                let key = get("property")
                    .or_else(|| get("name"))
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if let Some(content) = get("content") {
                    let slot = match key.as_str() {
                        "og:title" => &mut og_title,
                        "twitter:title" => &mut tw_title,
                        "og:description" => &mut og_description,
                        "twitter:description" => &mut tw_description,
                        "description" => &mut description,
                        "og:site_name" => &mut site_name,
                        "og:image" | "og:image:url" | "og:image:secure_url" => &mut og_image,
                        "twitter:image" | "twitter:image:src" => &mut tw_image,
                        _ => &mut None,
                    };
                    if slot.is_none() {
                        *slot = Some(content.to_owned());
                    }
                }
            }
            _ => {}
        }
        i = close;
    }

    let image = og_image
        .or(tw_image)
        .and_then(|raw| base.join(decode_entities(raw.trim()).as_str()).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https"));
    Meta {
        title: clean(og_title.or(tw_title).or(title_tag), TITLE_MAX),
        description: clean(
            og_description.or(tw_description).or(description),
            DESCRIPTION_MAX,
        ),
        site_name: clean(site_name, SITE_NAME_MAX),
        image,
    }
}

/// Index just past the `>` that ends the tag at `at`, quotes respected.
fn tag_end(lower: &str, at: usize) -> Option<usize> {
    let mut quote = None;
    for (n, c) in lower[at..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(at + n + 1),
            _ => {}
        }
    }
    None
}

/// `name="value"`, `name='value'` and `name=value`; names lowercased.
fn attributes(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = src.char_indices().peekable();
    loop {
        while chars
            .next_if(|(_, c)| c.is_whitespace() || *c == '/')
            .is_some()
        {}
        let Some(&(start, _)) = chars.peek() else {
            return out;
        };
        let mut end = start;
        while let Some((n, c)) =
            chars.next_if(|(_, c)| !c.is_whitespace() && *c != '=' && *c != '/')
        {
            end = n + c.len_utf8();
        }
        let name = src[start..end].to_ascii_lowercase();
        while chars.next_if(|(_, c)| c.is_whitespace()).is_some() {}
        if chars.next_if(|(_, c)| *c == '=').is_none() {
            if name.is_empty() {
                chars.next();
            }
            continue;
        }
        while chars.next_if(|(_, c)| c.is_whitespace()).is_some() {}
        let value = match chars.peek() {
            Some(&(n, q @ ('"' | '\''))) => {
                chars.next();
                let from = n + 1;
                let mut to = src.len();
                for (m, c) in chars.by_ref() {
                    if c == q {
                        to = m;
                        break;
                    }
                }
                &src[from..to]
            }
            Some(&(n, _)) => {
                let mut to = n;
                while let Some((m, c)) = chars.next_if(|(_, c)| !c.is_whitespace()) {
                    to = m + c.len_utf8();
                }
                &src[n..to]
            }
            None => "",
        };
        if !name.is_empty() {
            out.push((name, value.to_owned()));
        }
    }
}

/// Entities decoded, whitespace collapsed, control characters dropped,
/// cut at `max` characters; empty is `None`.
pub fn clean(raw: Option<String>, max: usize) -> Option<String> {
    let text = decode_entities(&raw?);
    let mut out = String::new();
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
        } else if !c.is_control() {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if out.chars().count() > max {
        out = out
            .chars()
            .take(max - 1)
            .collect::<String>()
            .trim_end()
            .to_owned();
        out.push('…');
    }
    (!out.is_empty()).then_some(out)
}

pub fn decode_entities(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = if let Some(num) = entity.strip_prefix('#') {
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => num.parse().ok(),
            };
            code.and_then(char::from_u32)
                .filter(|c| *c != '\0')
                .map(String::from)
        } else {
            named(entity).map(String::from)
        };
        match decoded {
            Some(text) => {
                out.push_str(&text);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named(entity: &str) -> Option<&'static str> {
    Some(match entity {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "ndash" => "–",
        "mdash" => "—",
        "hellip" => "…",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "laquo" => "«",
        "raquo" => "»",
        "middot" => "·",
        "bull" => "•",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "euro" => "€",
        "auml" => "ä",
        "ouml" => "ö",
        "uuml" => "ü",
        "Auml" => "Ä",
        "Ouml" => "Ö",
        "Uuml" => "Ü",
        "szlig" => "ß",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://site.example/a/page").unwrap()
    }

    #[test]
    fn prefers_opengraph_then_twitter_then_title() {
        let html = r#"<!doctype html><html><head>
            <title>Plain &amp; simple</title>
            <meta name="twitter:title" content="Tw title">
            <meta property='og:title' content="OG &quot;title&quot;" />
            <meta name=description content=Fallback>
            <meta property="og:site_name" content="Site">
            <meta property="og:image" content="/img/card.png">
            <script>var x = "<meta property='og:description' content='no'>";</script>
            </head><body><meta property="og:description" content="too late"></body>"#;
        let meta = parse(html, &base());
        assert_eq!(meta.title.as_deref(), Some("OG \"title\""));
        assert_eq!(meta.description.as_deref(), Some("Fallback"));
        assert_eq!(meta.site_name.as_deref(), Some("Site"));
        assert_eq!(
            meta.image.map(String::from).as_deref(),
            Some("https://site.example/img/card.png")
        );
    }

    #[test]
    fn falls_back_to_the_title_tag() {
        let meta = parse("<head><TITLE>\n  Hello\n  World </TITLE></head>", &base());
        assert_eq!(meta.title.as_deref(), Some("Hello World"));
        assert!(meta.image.is_none());
    }

    #[test]
    fn keeps_only_web_images_and_cuts_long_text() {
        let long = "x".repeat(1000);
        let html = format!(
            r#"<meta property="og:image" content="javascript:alert(1)"><meta property="og:description" content="{long}">"#
        );
        let meta = parse(&html, &base());
        assert!(meta.image.is_none());
        let description = meta.description.unwrap();
        assert_eq!(description.chars().count(), DESCRIPTION_MAX);
        assert!(description.ends_with('…'));
    }

    #[test]
    fn survives_broken_markup() {
        for html in [
            "<meta",
            "<meta content=\"x",
            "<!-- open",
            "<title>open",
            "<<<>>>",
            "<meta =x>",
        ] {
            let _ = parse(html, &base());
        }
    }

    #[test]
    fn decodes_entities_and_latin1() {
        assert_eq!(
            decode_entities("a &lt;b&gt; &#252; &#xFC; &bogus; & x"),
            "a <b> ü ü &bogus; & x"
        );
        assert_eq!(decode(b"caf\xe9", Some("iso-8859-1")), "café");
        assert!(decode(b"<meta charset=\"windows-1252\">caf\xe9", None).ends_with("café"));
        assert_eq!(decode("café".as_bytes(), None), "café");
    }
}
