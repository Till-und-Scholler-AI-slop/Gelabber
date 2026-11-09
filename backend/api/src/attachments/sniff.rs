//! Content check of an upload (issue #255). The browser names the type at
//! presign; before the file joins a message, its first bytes must agree.
//! Text types only need to be text in any encoding: a renamed `.html`
//! stays harmless, because text is always served as `text/plain` and as a
//! download.

/// Bytes read from the start of an object; `ustar` sits at offset 257.
pub const PREFIX_LEN: usize = 512;
/// A JPEG's frame header comes after its EXIF, XMP and ICC segments (up to
/// 64 KiB each), so more of it is read to find the size.
const JPEG_PREFIX_LEN: usize = 256 * 1024;
/// Images shown inline must decode in every viewer's browser: at most this
/// many pixels a side, and 8192² in all.
const IMAGE_EDGE_MAX: u32 = 16384;
const IMAGE_PIXELS_MAX: u64 = 1 << 26;

/// How many leading bytes `matches` and `too_many_pixels` want.
pub fn prefix_len(content_type: &str) -> usize {
    if content_type == "image/jpeg" {
        JPEG_PREFIX_LEN
    } else {
        PREFIX_LEN
    }
}

/// Whether an image declares a canvas past the bounds. A size the prefix
/// does not show is let through: the check cannot prove it.
pub fn too_many_pixels(content_type: &str, head: &[u8]) -> bool {
    let size = match content_type {
        // The logical screen; browsers draw every frame within it.
        "image/gif" => head.get(6..10).map(|b| {
            (
                u32::from(u16::from_le_bytes([b[0], b[1]])),
                u32::from(u16::from_le_bytes([b[2], b[3]])),
            )
        }),
        t if t.starts_with("image/") => crate::image_size::dimensions(head),
        _ => None,
    };
    size.is_some_and(|(width, height)| {
        width > IMAGE_EDGE_MAX
            || height > IMAGE_EDGE_MAX
            || u64::from(width) * u64::from(height) > IMAGE_PIXELS_MAX
    })
}

pub fn matches(content_type: &str, head: &[u8]) -> bool {
    let at = |offset: usize, magic: &[u8]| head.get(offset..offset + magic.len()) == Some(magic);
    match content_type {
        "image/jpeg" => at(0, &[0xFF, 0xD8, 0xFF]),
        "image/png" => at(0, b"\x89PNG\r\n\x1a\n"),
        "image/gif" => at(0, b"GIF87a") || at(0, b"GIF89a"),
        "image/webp" => at(0, b"RIFF") && at(8, b"WEBP"),
        "application/pdf" => at(0, b"%PDF-"),
        "application/zip" => at(0, b"PK\x03\x04") || at(0, b"PK\x05\x06"),
        "application/gzip" => at(0, &[0x1F, 0x8B]),
        "application/x-7z-compressed" => at(0, &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]),
        "application/x-tar" => at(257, b"ustar"),
        "audio/mpeg" => at(0, b"ID3") || mpeg_frame(head),
        "audio/aac" => at(0, b"ID3") || adts_frame(head),
        "audio/wav" => at(0, b"RIFF") && at(8, b"WAVE"),
        "audio/ogg" => at(0, b"OggS"),
        "audio/flac" => at(0, b"fLaC"),
        "audio/webm" | "video/webm" => at(0, &[0x1A, 0x45, 0xDF, 0xA3]),
        "audio/mp4" | "video/mp4" => at(4, b"ftyp"),
        // Older QuickTime files may open with another top-level atom.
        "video/quicktime" => [b"ftyp", b"moov", b"mdat", b"wide", b"free", b"skip"]
            .iter()
            .any(|atom| at(4, *atom)),
        t if super::validate::is_text(t) => is_text(head),
        _ => false,
    }
}

/// MPEG audio frame sync: eleven set bits, then a valid layer.
fn mpeg_frame(head: &[u8]) -> bool {
    matches!(head, [0xFF, b, ..] if b & 0xE0 == 0xE0 && b & 0x06 != 0)
}

/// AAC ADTS sync: twelve set bits, layer zero.
fn adts_frame(head: &[u8]) -> bool {
    matches!(head, [0xFF, b, ..] if b & 0xF6 == 0xF0)
}

/// Text in any encoding: no NUL byte, or UTF-16 with its byte order mark
/// (Excel's "Unicode text"). Windows-1252 or Latin-1 CSVs and logs pass;
/// the preview picks the encoding when it shows them.
fn is_text(head: &[u8]) -> bool {
    head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) || !head.contains(&0)
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn magic_bytes_must_match_the_declared_type() {
        assert!(matches("image/png", b"\x89PNG\r\n\x1a\n...."));
        assert!(!matches("image/png", b"\xFF\xD8\xFF\xE0"));
        assert!(matches("image/webp", b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(!matches("image/webp", b"RIFF\0\0\0\0WAVEfmt "));
        assert!(matches("audio/wav", b"RIFF\0\0\0\0WAVEfmt "));
        assert!(matches("video/mp4", b"\0\0\0\x20ftypisom"));
        assert!(matches("video/quicktime", b"\0\0\0\x14ftypqt  "));
        assert!(matches("video/webm", &[0x1A, 0x45, 0xDF, 0xA3, 1]));
        assert!(matches("audio/mpeg", &[0xFF, 0xFB, 0x90, 0x64]));
        assert!(matches("audio/mpeg", b"ID3\x04"));
        assert!(!matches("audio/mpeg", b"OggS"));
        assert!(matches("audio/aac", &[0xFF, 0xF1, 0x50, 0x80]));
        let mut tar = vec![0u8; 300];
        tar[257..262].copy_from_slice(b"ustar");
        assert!(matches("application/x-tar", &tar));
        assert!(!matches("application/x-tar", &[0u8; 300]));
        assert!(!matches("image/jpeg", &[0xFF]));
    }

    #[test]
    fn text_is_any_encoding_without_nul_and_html_text_stays_text() {
        assert!(matches("text/plain", b"fn main() {}\n"));
        assert!(matches("text/plain", b"<!doctype html><script>x</script>"));
        assert!(matches("application/json", b"{\"a\": 1}"));
        assert!(matches("text/markdown", b"# Titel \xC3"));
        // Excel CSV on a German system: "Größe" in Windows-1252.
        assert!(matches("text/csv", b"Gr\xF6\xDFe;Preis\n"));
        // UTF-16 LE with BOM: "a,b".
        assert!(matches("text/csv", b"\xFF\xFEa\0,\0b\0"));
        assert!(!matches("text/plain", b"MZ\x90\0\x03"));
        assert!(!matches("text/plain", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
    }
}
