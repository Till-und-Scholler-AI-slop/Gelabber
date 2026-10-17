//! Content check of an upload (issue #255). The browser names the type at
//! presign; before the file joins a message, its first bytes must agree.
//! Text types only need to be text: a renamed `.html` stays harmless,
//! because text is always served as `text/plain` and as a download.

/// Bytes read from the start of an object; `ustar` sits at offset 257.
pub const PREFIX_LEN: usize = 512;

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

/// UTF-8 without NUL. The prefix may end inside a character.
fn is_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(err) => err.error_len().is_none() && head.len() - err.valid_up_to() < 4,
    }
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
    fn text_is_utf8_without_nul_and_html_text_stays_text() {
        assert!(matches("text/plain", b"fn main() {}\n"));
        assert!(matches("text/plain", b"<!doctype html><script>x</script>"));
        assert!(matches("application/json", b"{\"a\": 1}"));
        // Cut inside "ä" (C3 A4) at the end of the prefix.
        assert!(matches("text/markdown", b"# Titel \xC3"));
        assert!(!matches("text/plain", b"MZ\x90\0\x03"));
        assert!(!matches("text/csv", b"a,b\n\xFF\xFE"));
    }
}
