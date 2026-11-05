//! Klipy response shapes → [`Gif`]. Read leniently from JSON values: an ad,
//! a missing size or a URL off Klipy's media hosts drops that one item,
//! never the page.

use serde_json::Value;
use url::Url;

use super::{Gif, GifPage, PROVIDER};

/// Which rendition an item is reduced to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// Picker grid: about 220 px wide.
    Preview,
    /// Inline in a message: the full rendition.
    Message,
}

impl Size {
    fn order(self) -> &'static [&'static str] {
        match self {
            Self::Preview => &["sm", "md", "hd", "xs"],
            Self::Message => &["hd", "md", "sm"],
        }
    }
}

pub const SLUG_MAX: usize = 200;
const TITLE_MAX: usize = 200;
const DIMENSION_MAX: u64 = 4096;

pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= SLUG_MAX
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `{ result, data: { data: [...], has_next } }` from search/trending.
pub fn page(body: &Value, page: u32, size: Size, origins: &[String]) -> GifPage {
    let data = &body["data"];
    let gifs = data["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| gif(item, size, origins))
                .collect()
        })
        .unwrap_or_default();
    let has_next = data["has_next"].as_bool().unwrap_or(false);
    GifPage {
        gifs,
        next_page: has_next.then_some(page + 1),
    }
}

/// `gifs/items?slugs=` answers `{ data: { data: [...] } }`; a bare
/// `data: [...]` is accepted as well.
pub fn items(body: &Value, size: Size, origins: &[String]) -> Vec<Gif> {
    let data = &body["data"];
    data["data"]
        .as_array()
        .or_else(|| data.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| gif(item, size, origins))
                .collect()
        })
        .unwrap_or_default()
}

pub fn ok(body: &Value) -> bool {
    body["result"].as_bool() != Some(false)
}

fn gif(item: &Value, size: Size, origins: &[String]) -> Option<Gif> {
    // Ads (`type: "ad"`) are HTML for a WebView; Gelabber does not render them.
    if item["type"].as_str().is_some_and(|kind| kind != "gif") {
        return None;
    }
    let slug = item["slug"].as_str().filter(|slug| valid_slug(slug))?;
    let title: String = item["title"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(TITLE_MAX)
        .collect();
    let file = item["file"].as_object()?;
    size.order().iter().find_map(|name| {
        let set = file.get(*name)?;
        let animated = media(&set["webp"], origins).or_else(|| media(&set["gif"], origins))?;
        let still = media(&set["jpg"], origins).map(|still| still.url);
        Some(Gif {
            provider: PROVIDER.to_owned(),
            slug: slug.to_owned(),
            title: title.trim().to_owned(),
            width: animated.width,
            height: animated.height,
            url: animated.url,
            still,
        })
    })
}

struct Media {
    url: String,
    width: u32,
    height: u32,
}

fn media(value: &Value, origins: &[String]) -> Option<Media> {
    let url = value["url"].as_str()?;
    if !media_allowed(url, origins) {
        return None;
    }
    let dimension = |key: &str| {
        value[key]
            .as_u64()
            .filter(|n| (1..=DIMENSION_MAX).contains(n))
            .map(|n| n as u32)
    };
    Some(Media {
        url: url.to_owned(),
        width: dimension("width")?,
        height: dimension("height")?,
    })
}

/// An absolute URL on one of the configured media origins, without
/// credentials. The only URLs `/api/gifs/media` fetches.
pub fn media_allowed(raw: &str, origins: &[String]) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    origins.contains(&url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn origins() -> Vec<String> {
        vec!["https://static.klipy.com".to_owned()]
    }

    fn rendition(name: &str, w: u32) -> Value {
        json!({
            "webp": { "url": format!("https://static.klipy.com/ii/a/{name}.webp"), "width": w, "height": w, "size": 1 },
            "gif": { "url": format!("https://static.klipy.com/ii/a/{name}.gif"), "width": w, "height": w, "size": 1 },
            "jpg": { "url": format!("https://static.klipy.com/ii/a/{name}.jpg"), "width": w, "height": w, "size": 1 },
        })
    }

    fn item(slug: &str) -> Value {
        json!({
            "id": 8041071659142944_u64,
            "slug": slug,
            "title": "Hello",
            "type": "gif",
            "file": { "hd": rendition("hd", 498), "sm": rendition("sm", 220) },
        })
    }

    #[test]
    fn picks_the_rendition_for_each_use() {
        let body =
            json!({ "result": true, "data": { "data": [item("hello-hi-662")], "has_next": true } });
        let preview = page(&body, 1, Size::Preview, &origins());
        assert_eq!(preview.next_page, Some(2));
        let gif = &preview.gifs[0];
        assert_eq!(gif.slug, "hello-hi-662");
        assert_eq!(gif.url, "https://static.klipy.com/ii/a/sm.webp");
        assert_eq!(
            gif.still.as_deref(),
            Some("https://static.klipy.com/ii/a/sm.jpg")
        );
        assert_eq!((gif.width, gif.height), (220, 220));
        let full = items(
            &json!({ "data": { "data": [item("hello-hi-662")] } }),
            Size::Message,
            &origins(),
        );
        assert_eq!(full[0].url, "https://static.klipy.com/ii/a/hd.webp");
        assert_eq!(full[0].provider, "klipy");
    }

    #[test]
    fn drops_ads_foreign_hosts_and_bad_slugs_but_keeps_the_page() {
        let mut foreign = item("foreign");
        foreign["file"] = json!({ "hd": { "webp": { "url": "https://evil.example/x.webp", "width": 10, "height": 10 } } });
        let ad = json!({ "type": "ad", "content": "<div></div>" });
        let body = json!({ "result": true, "data": {
            "data": [ad, foreign, item("../etc"), item("fine")], "has_next": false
        } });
        let result = page(&body, 3, Size::Preview, &origins());
        assert_eq!(result.next_page, None);
        assert_eq!(result.gifs.len(), 1);
        assert_eq!(result.gifs[0].slug, "fine");
    }

    #[test]
    fn falls_back_to_gif_when_there_is_no_webp() {
        let mut only_gif = item("only-gif");
        only_gif["file"]["sm"]
            .as_object_mut()
            .unwrap()
            .remove("webp");
        let result = page(
            &json!({ "data": { "data": [only_gif] } }),
            1,
            Size::Preview,
            &origins(),
        );
        assert_eq!(result.gifs[0].url, "https://static.klipy.com/ii/a/sm.gif");
    }

    #[test]
    fn media_must_be_on_an_allowed_origin() {
        let o = origins();
        assert!(media_allowed("https://static.klipy.com/ii/a/b.webp", &o));
        assert!(!media_allowed("http://static.klipy.com/ii/a/b.webp", &o));
        assert!(!media_allowed("https://static.klipy.com:8443/a.webp", &o));
        assert!(!media_allowed(
            "https://user:pw@static.klipy.com/a.webp",
            &o
        ));
        assert!(!media_allowed(
            "https://static.klipy.com.evil.example/a.webp",
            &o
        ));
        assert!(!media_allowed("/api/gifs/media", &o));
        assert!(!media_allowed("https://127.0.0.1/a.webp", &o));
        assert!(!media_allowed("https://[::1]/a.webp", &o));
        assert!(!media_allowed("https://[::ffff:127.0.0.1]/a.webp", &o));
        // A trailing dot is another origin string, so it is refused too.
        assert!(!media_allowed("https://static.klipy.com./a.webp", &o));
        // Look-alike hosts become punycode and never match.
        assert!(!media_allowed("https://ѕtatic.klipy.com/a.webp", &o));
        assert!(!media_allowed(
            "https://static.klipy.com%2eevil.example/a.webp",
            &o
        ));
        assert!(!media_allowed(
            "https://static.klipy.com@evil.example/a.webp",
            &o
        ));
        // Case and an escaped dot normalise to the allowed host itself.
        assert!(media_allowed("HTTPS://STATIC.KLIPY.COM/ii/a/b.webp", &o));
        assert!(media_allowed("https://static%2Eklipy.com/ii/a/b.webp", &o));
        assert!(media_allowed(
            "https://static.klipy.com:443/ii/a/b.webp",
            &o
        ));
    }

    #[test]
    fn result_false_is_not_ok() {
        assert!(ok(&json!({ "result": true })));
        assert!(!ok(&json!({ "result": false })));
    }
}
