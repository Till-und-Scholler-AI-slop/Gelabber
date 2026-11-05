//! Pixel dimensions of uploaded PNG, JPEG and WebP images, read from their
//! headers without decoding. A file under the byte limit can still declare
//! a huge canvas (a 15000×15000 PNG compresses to well under 1 MiB and
//! decodes to ~860 MiB RGBA), and every viewer's browser would decode it.
//! Uploads and downloads of profile images therefore check the size the
//! decoder will use, not just the byte count.

/// Width and height as the image header declares them, or `None` when the
/// header is missing, truncated or of a kind browsers would not decode the
/// same way (unsupported JPEG process, chunk other than the first frame).
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let (width, height) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)?
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg(bytes)?
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        webp(bytes)?
    } else {
        return None;
    };
    (width > 0 && height > 0).then_some((width, height))
}

/// Whether the declared size fits `max_width`×`max_height`.
pub fn fits(bytes: &[u8], max_width: u32, max_height: u32) -> bool {
    dimensions(bytes).is_some_and(|(w, h)| w <= max_width && h <= max_height)
}

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_be_bytes([b[0], b[1]])))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_le_bytes([b[0], b[1]])))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], 0]))
}

/// IHDR must be the first chunk; APNG frames must fit inside it.
fn png(bytes: &[u8]) -> Option<(u32, u32)> {
    if be32(bytes, 8)? != 13 || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be32(bytes, 16)?, be32(bytes, 20)?))
}

/// The first frame header before any scan. Only the processes browsers
/// decode: baseline (SOF0), extended sequential (SOF1) and progressive
/// (SOF2). A height of 0 (defined later by a DNL marker) is refused.
fn jpeg(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xff {
            return None;
        }
        // Any number of 0xff fill bytes may precede a marker.
        while *bytes.get(at)? == 0xff {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        match marker {
            // Standalone markers carry no length.
            0x01 | 0xd0..=0xd7 => continue,
            // End of image or start of scan before any frame header.
            0xd9 | 0xda => return None,
            _ => {}
        }
        let length = be16(bytes, at)? as usize;
        if length < 2 {
            return None;
        }
        match marker {
            0xc0..=0xc2 => {
                // length, precision, height, width
                return Some((be16(bytes, at + 5)?, be16(bytes, at + 3)?));
            }
            // Other frame types (lossless, hierarchical, arithmetic) and
            // DHP: not something a canvas encoder produces.
            0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf | 0xde => return None,
            _ => at += length,
        }
    }
}

/// The first chunk decides the format: `VP8 ` (lossy), `VP8L` (lossless)
/// or `VP8X` (extended; its canvas bounds every frame and the alpha plane).
fn webp(bytes: &[u8]) -> Option<(u32, u32)> {
    let fourcc = bytes.get(12..16)?;
    let data = 20;
    match fourcc {
        b"VP8 " => {
            if bytes.get(data + 3..data + 6)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            // 14 bits of size; the top two are an upscaling hint the
            // decoder ignores.
            Some((
                le16(bytes, data + 6)? & 0x3fff,
                le16(bytes, data + 8)? & 0x3fff,
            ))
        }
        b"VP8L" => {
            if *bytes.get(data)? != 0x2f {
                return None;
            }
            let b = bytes.get(data + 1..data + 5)?;
            let bits = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => Some((le24(bytes, data + 4)? + 1, le24(bytes, data + 7)? + 1)),
        _ => None,
    }
}

/// Minimal headers with a given size, for tests here and in `api/tests`.
#[doc(hidden)]
pub mod fixtures {
    /// PNG signature and IHDR (RGBA, 8 bit); no pixel data.
    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    /// RIFF/WEBP with a lossy `VP8 ` key frame header; no pixel data.
    pub fn webp(width: u16, height: u16) -> Vec<u8> {
        let mut chunk = vec![0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a];
        chunk.extend_from_slice(&width.to_le_bytes());
        chunk.extend_from_slice(&height.to_le_bytes());
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(4 + 8 + chunk.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WEBPVP8 ");
        bytes.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&chunk);
        bytes
    }

    /// SOI, an APP0 segment and a baseline SOF0; no scan.
    pub fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00];
        bytes.extend_from_slice(&[0xff, 0xff, 0xc0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_png_jpeg_and_webp_headers() {
        assert_eq!(dimensions(&fixtures::png(256, 256)), Some((256, 256)));
        assert_eq!(
            dimensions(&fixtures::png(15000, 15000)),
            Some((15000, 15000))
        );
        assert_eq!(dimensions(&fixtures::jpeg(960, 320)), Some((960, 320)));
        assert_eq!(dimensions(&fixtures::webp(960, 320)), Some((960, 320)));
        // VP8L: signature, then (width-1) and (height-1) in 14 bits each.
        let bits: u32 = 255 | (127 << 14);
        let mut lossless = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
        lossless.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(dimensions(&lossless), Some((256, 128)));
        // VP8X: flags, reserved, then 24-bit (width-1) and (height-1).
        let mut extended = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\x10\0\0\0".to_vec();
        extended.extend_from_slice(&[0x97, 0x3a, 0x00, 0x96, 0x3a, 0x00]);
        assert_eq!(dimensions(&extended), Some((15000, 14999)));
    }

    #[test]
    fn webp_scale_bits_do_not_count() {
        let mut bytes = fixtures::webp(100, 50);
        // Set both upscaling hints.
        bytes[27] |= 0xc0;
        bytes[29] |= 0xc0;
        assert_eq!(dimensions(&bytes), Some((100, 50)));
    }

    #[test]
    fn refuses_what_it_cannot_prove() {
        assert_eq!(dimensions(b"\x89PNG\r\n\x1a\nnot really pixels"), None);
        assert_eq!(dimensions(b"RIFF\x10\0\0\0WEBPVP8 pixels"), None);
        assert_eq!(dimensions(&fixtures::png(0, 10)), None);
        assert_eq!(dimensions(&fixtures::png(256, 256)[..20]), None);
        // IHDR not first.
        let mut late = fixtures::png(1, 1);
        late[12..16].copy_from_slice(b"tEXt");
        assert_eq!(dimensions(&late), None);
        // JPEG: scan before frame, unsupported process, height from DNL.
        assert_eq!(dimensions(&[0xff, 0xd8, 0xff, 0xda, 0x00, 0x02]), None);
        let mut lossless = fixtures::jpeg(10, 10);
        lossless[10] = 0xc3;
        assert_eq!(dimensions(&lossless), None);
        assert_eq!(dimensions(&fixtures::jpeg(10, 0)), None);
        assert_eq!(dimensions(&[0xff, 0xd8, 0xff]), None);
        assert_eq!(dimensions(&[0xff, 0xd8, 0x00, 0x00]), None);
        // An APP segment that claims to run past the end.
        assert_eq!(
            dimensions(&[0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff, 0x00]),
            None
        );
        assert_eq!(
            dimensions(b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0\0\0\0\0"),
            None
        );
        assert_eq!(dimensions(b"RIFF\0\0\0\0WEBPALPH\0\0\0\0"), None);
        assert_eq!(dimensions(b"GIF89a"), None);
    }

    #[test]
    fn fits_checks_both_edges() {
        assert!(fits(&fixtures::png(256, 256), 256, 256));
        assert!(fits(&fixtures::png(64, 64), 256, 256));
        assert!(!fits(&fixtures::png(257, 256), 256, 256));
        assert!(!fits(&fixtures::png(256, 257), 256, 256));
        assert!(!fits(&fixtures::webp(960, 321), 960, 320));
        assert!(!fits(b"\x89PNG\r\n\x1a\n", 256, 256));
    }
}
