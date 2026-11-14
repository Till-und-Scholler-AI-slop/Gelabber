//! Privacy metadata of uploaded pictures (#574), handled without decoding.
//!
//! Phones write the GPS position, the camera's serial number and editing
//! history into their photos, and every member who can see a message can
//! download its original. Before a picture joins a message, [`strip`]
//! rewrites it without that metadata. The pixels stay byte for byte as
//! uploaded (no re-encode): only metadata containers are dropped. Kept are
//! the parts a viewer needs to show the picture as intended: the EXIF
//! orientation (rewritten as the only EXIF entry), the ICC colour profile,
//! and JPEG's JFIF and Adobe segments that say how to read the colours.
//!
//! Dropped per format:
//! - JPEG: EXIF (except the orientation), XMP, Photoshop/IPTC and every
//!   other application segment, comments, the MPF index and everything
//!   after the main picture's end, where phones append gain maps and the
//!   video of a "motion photo" (which carries its own location).
//! - PNG: `eXIf` (except the orientation), `tEXt`, `zTXt`, `iTXt` (where
//!   XMP lives) and `tIME`.
//! - WebP: the `EXIF` (except the orientation) and `XMP ` chunks.
//! - GIF stays as it is: it has no EXIF, and cameras do not write it.
//!
//! A file these parsers do not understand is left as it is; the content
//! check (`sniff`) already decided whether it may be attached at all.

/// EXIF orientation 1..=8 (1 = as stored), or 1 when there is none.
pub fn orientation(content_type: &str, bytes: &[u8]) -> u8 {
    let tiff = match content_type {
        "image/jpeg" => jpeg_exif(bytes),
        "image/png" => png_chunks(bytes).and_then(|chunks| {
            chunks
                .into_iter()
                .find(|c| &c.kind == b"eXIf")
                .map(|c| c.data)
        }),
        "image/webp" => webp_chunks(bytes).and_then(|chunks| {
            chunks
                .into_iter()
                .find(|c| &c.kind == b"EXIF")
                .map(|c| c.data)
        }),
        _ => None,
    };
    tiff.and_then(tiff_orientation).unwrap_or(1)
}

/// Width and height as shown: the header's size, turned by the EXIF
/// orientation. `None` when the header does not say.
pub fn display_size(content_type: &str, bytes: &[u8]) -> Option<(u32, u32)> {
    let (width, height) = match content_type {
        "image/gif" => super::sniff::gif_size(bytes)?,
        "image/jpeg" | "image/png" | "image/webp" => crate::image_size::dimensions(bytes)?,
        _ => return None,
    };
    if (width, height) == (0, 0) {
        return None;
    }
    Some(if orientation(content_type, bytes) >= 5 {
        (height, width)
    } else {
        (width, height)
    })
}

/// Whether a GIF, PNG (APNG) or WebP holds more than one frame. Browsers
/// play those; a thumbnail shows only the first frame.
pub fn animated(content_type: &str, bytes: &[u8]) -> bool {
    match content_type {
        "image/gif" => gif_frames(bytes) > 1,
        "image/png" => png_chunks(bytes).is_some_and(|chunks| {
            // `acTL` before the first `IDAT` makes it an APNG.
            chunks
                .iter()
                .take_while(|c| &c.kind != b"IDAT")
                .any(|c| &c.kind == b"acTL")
        }),
        "image/webp" => webp_chunks(bytes).is_some_and(|chunks| {
            chunks.first().is_some_and(|c| {
                &c.kind == b"VP8X" && c.data.first().is_some_and(|f| f & 0x02 != 0)
            })
        }),
        _ => false,
    }
}

/// The picture without privacy metadata, or `None` when there was none to
/// remove (or the file is not one these parsers read).
pub fn strip(content_type: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    let stripped = match content_type {
        "image/jpeg" => strip_jpeg(bytes)?,
        "image/png" => strip_png(bytes)?,
        "image/webp" => strip_webp(bytes)?,
        _ => return None,
    };
    (stripped != bytes).then_some(stripped)
}

// --- EXIF (TIFF) ---------------------------------------------------------

const ORIENTATION_TAG: u16 = 0x0112;

/// Orientation from a TIFF structure (`II*\0` or `MM\0*`), IFD0 only.
fn tiff_orientation(tiff: &[u8]) -> Option<u8> {
    let little = match tiff.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let b = tiff.get(at..at + 2)?;
        Some(if little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let u32_at = |at: usize| {
        let b = tiff.get(at..at + 4)?;
        Some(if little {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    for index in 0..count.min(512) {
        let entry = ifd + 2 + index * 12;
        if u16_at(entry)? == ORIENTATION_TAG {
            // SHORT, count 1: the value sits in the first two value bytes.
            if u16_at(entry + 2)? != 3 {
                return None;
            }
            let value = u16_at(entry + 8)?;
            return (1..=8).contains(&value).then_some(value as u8);
        }
    }
    None
}

/// A TIFF structure whose only entry is the orientation.
fn orientation_tiff(orientation: u8) -> Vec<u8> {
    let mut tiff = Vec::with_capacity(26);
    tiff.extend_from_slice(b"II*\0");
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&ORIENTATION_TAG.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&u16::from(orientation).to_le_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff
}

/// The orientation worth keeping (2..=8); 1 needs no entry.
fn kept_orientation(tiff: &[u8]) -> Option<u8> {
    tiff_orientation(tiff).filter(|&o| o != 1)
}

// --- JPEG ----------------------------------------------------------------

const EXIF_HEADER: &[u8] = b"Exif\0\0";
const ICC_HEADER: &[u8] = b"ICC_PROFILE\0";

struct Segment<'a> {
    marker: u8,
    /// The whole segment, marker and length included.
    raw: &'a [u8],
    payload: &'a [u8],
}

/// The segments up to and including the main picture's end (EOI), with
/// the entropy-coded data of each scan attached to its SOS segment.
/// `None` for anything that is not a well-formed JPEG.
fn jpeg_segments(bytes: &[u8]) -> Option<Vec<Segment<'_>>> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut segments = vec![Segment {
        marker: 0xD8,
        raw: &bytes[..2],
        payload: &[],
    }];
    let mut at = 2;
    loop {
        let start = at;
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        while *bytes.get(at)? == 0xFF {
            at += 1;
        }
        let marker = bytes[at];
        at += 1;
        match marker {
            0xD9 => {
                segments.push(Segment {
                    marker,
                    raw: &bytes[start..at],
                    payload: &[],
                });
                return Some(segments);
            }
            0x01 | 0xD0..=0xD7 => {
                segments.push(Segment {
                    marker,
                    raw: &bytes[start..at],
                    payload: &[],
                });
                continue;
            }
            0x00 | 0xD8 => return None,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        if length < 2 {
            return None;
        }
        let end = at + length;
        let payload = bytes.get(at + 2..end)?;
        at = end;
        if marker == 0xDA {
            // Entropy-coded data: up to the next marker other than a
            // stuffed zero, a restart marker or fill bytes.
            loop {
                let ff = at + bytes.get(at..)?.iter().position(|&b| b == 0xFF)?;
                let mut next = ff + 1;
                while *bytes.get(next)? == 0xFF {
                    next += 1;
                }
                match bytes[next] {
                    0x00 | 0xD0..=0xD7 => at = next + 1,
                    _ => {
                        at = ff;
                        break;
                    }
                }
            }
        }
        segments.push(Segment {
            marker,
            raw: &bytes[start..at],
            payload,
        });
    }
}

fn jpeg_exif(bytes: &[u8]) -> Option<&[u8]> {
    jpeg_segments(bytes)?
        .into_iter()
        .take_while(|s| s.marker != 0xDA)
        .find(|s| s.marker == 0xE1 && s.payload.starts_with(EXIF_HEADER))
        .map(|s| &s.payload[EXIF_HEADER.len()..])
}

fn strip_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let segments = jpeg_segments(bytes)?;
    let mut out = Vec::with_capacity(bytes.len());
    let mut wrote_orientation = false;
    for segment in &segments {
        let keep = match segment.marker {
            // JFIF / JFXX.
            0xE0 => {
                segment.payload.starts_with(b"JFIF\0") || segment.payload.starts_with(b"JFXX\0")
            }
            0xE1 => {
                if !wrote_orientation
                    && let Some(o) = segment
                        .payload
                        .strip_prefix(EXIF_HEADER)
                        .and_then(kept_orientation)
                {
                    let tiff = orientation_tiff(o);
                    let length = (2 + EXIF_HEADER.len() + tiff.len()) as u16;
                    out.extend_from_slice(&[0xFF, 0xE1]);
                    out.extend_from_slice(&length.to_be_bytes());
                    out.extend_from_slice(EXIF_HEADER);
                    out.extend_from_slice(&tiff);
                    wrote_orientation = true;
                }
                false
            }
            // ICC profile only; MPF (the index of appended pictures) goes.
            0xE2 => segment.payload.starts_with(ICC_HEADER),
            // Adobe: says whether the colours are RGB or YCbCr.
            0xEE => segment.payload.starts_with(b"Adobe"),
            0xE3..=0xED | 0xEF | 0xFE => false,
            _ => true,
        };
        if keep {
            out.extend_from_slice(segment.raw);
        }
    }
    Some(out)
}

// --- PNG -----------------------------------------------------------------

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

struct Chunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
    raw: &'a [u8],
}

fn png_chunks(bytes: &[u8]) -> Option<Vec<Chunk<'_>>> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return None;
    }
    let mut at = PNG_SIGNATURE.len();
    let mut chunks = Vec::new();
    while at < bytes.len() {
        let length = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind: [u8; 4] = bytes.get(at + 4..at + 8)?.try_into().ok()?;
        let end = at.checked_add(12)?.checked_add(length)?;
        let data = bytes.get(at + 8..at + 8 + length)?;
        let raw = bytes.get(at..end)?;
        chunks.push(Chunk { kind, data, raw });
        at = end;
        if &kind == b"IEND" {
            break;
        }
    }
    chunks
        .last()
        .is_some_and(|c| &c.kind == b"IEND")
        .then_some(chunks)
}

fn strip_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let chunks = png_chunks(bytes)?;
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(PNG_SIGNATURE);
    for chunk in &chunks {
        match &chunk.kind {
            b"eXIf" => {
                if let Some(o) = kept_orientation(chunk.data) {
                    let tiff = orientation_tiff(o);
                    out.extend_from_slice(&(tiff.len() as u32).to_be_bytes());
                    let start = out.len();
                    out.extend_from_slice(b"eXIf");
                    out.extend_from_slice(&tiff);
                    let crc = crc32(&out[start..]);
                    out.extend_from_slice(&crc.to_be_bytes());
                }
            }
            b"tEXt" | b"zTXt" | b"iTXt" | b"tIME" => {}
            _ => out.extend_from_slice(chunk.raw),
        }
    }
    Some(out)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

// --- WebP ----------------------------------------------------------------

/// VP8X flags for the chunks this module touches.
const WEBP_EXIF_FLAG: u8 = 0x08;
const WEBP_XMP_FLAG: u8 = 0x04;

fn webp_chunks(bytes: &[u8]) -> Option<Vec<Chunk<'_>>> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    let riff_end =
        8usize.checked_add(u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize)?;
    let end_all = riff_end.min(bytes.len());
    let mut at = 12;
    let mut chunks = Vec::new();
    while at + 8 <= end_all {
        let kind: [u8; 4] = bytes[at..at + 4].try_into().ok()?;
        let length = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let data = bytes.get(at + 8..at.checked_add(8)?.checked_add(length)?)?;
        let padded = at + 8 + length + (length & 1);
        let raw = &bytes[at..padded.min(bytes.len())];
        chunks.push(Chunk { kind, data, raw });
        at = padded;
    }
    (!chunks.is_empty()).then_some(chunks)
}

fn strip_webp(bytes: &[u8]) -> Option<Vec<u8>> {
    let chunks = webp_chunks(bytes)?;
    // Only the extended format (VP8X) carries metadata chunks.
    if chunks.first().map(|c| &c.kind) != Some(b"VP8X") {
        return None;
    }
    let mut body = Vec::with_capacity(bytes.len());
    let mut exif_kept = false;
    for chunk in &chunks {
        match &chunk.kind {
            b"EXIF" => {
                if let Some(o) = kept_orientation(chunk.data) {
                    let tiff = orientation_tiff(o);
                    body.extend_from_slice(b"EXIF");
                    body.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
                    body.extend_from_slice(&tiff);
                    exif_kept = true;
                }
            }
            b"XMP " => {}
            _ => {
                body.extend_from_slice(chunk.raw);
                if chunk.raw.len() < 8 + chunk.data.len() + (chunk.data.len() & 1) {
                    // A last chunk cut short of its padding.
                    body.push(0);
                }
            }
        }
    }
    // VP8X's flags are the first byte of its data, right after the header.
    let mut flags = body[8] & !(WEBP_EXIF_FLAG | WEBP_XMP_FLAG);
    if exif_kept {
        flags |= WEBP_EXIF_FLAG;
    }
    body[8] = flags;
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    Some(out)
}

// --- GIF -----------------------------------------------------------------

/// Image descriptors up to the trailer, stopping at two.
fn gif_frames(bytes: &[u8]) -> usize {
    let Some(&flags) = bytes.get(10) else {
        return 0;
    };
    let mut at = 13;
    if flags & 0x80 != 0 {
        at += 3 << ((flags & 0x07) + 1);
    }
    let skip_blocks = |mut at: usize| -> Option<usize> {
        loop {
            let len = usize::from(*bytes.get(at)?);
            at += 1 + len;
            if len == 0 {
                return Some(at);
            }
        }
    };
    let mut frames = 0;
    while frames < 2 {
        match bytes.get(at) {
            Some(0x21) => match skip_blocks(at + 2) {
                Some(next) => at = next,
                None => break,
            },
            Some(0x2C) => {
                frames += 1;
                let Some(&local) = bytes.get(at + 9) else {
                    break;
                };
                at += 10;
                if local & 0x80 != 0 {
                    at += 3 << ((local & 0x07) + 1);
                }
                // LZW minimum code size, then the data sub-blocks.
                match skip_blocks(at + 1) {
                    Some(next) => at = next,
                    None => break,
                }
            }
            _ => break,
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exif_tiff(orientation: u16, with_gps: bool) -> Vec<u8> {
        // Big-endian, IFD0 with orientation and (optionally) a GPS pointer.
        let entries: u16 = if with_gps { 2 } else { 1 };
        let mut tiff = b"MM\0*".to_vec();
        tiff.extend_from_slice(&8u32.to_be_bytes());
        tiff.extend_from_slice(&entries.to_be_bytes());
        tiff.extend_from_slice(&0x0112u16.to_be_bytes());
        tiff.extend_from_slice(&3u16.to_be_bytes());
        tiff.extend_from_slice(&1u32.to_be_bytes());
        tiff.extend_from_slice(&orientation.to_be_bytes());
        tiff.extend_from_slice(&[0, 0]);
        if with_gps {
            tiff.extend_from_slice(&0x8825u16.to_be_bytes());
            tiff.extend_from_slice(&4u16.to_be_bytes());
            tiff.extend_from_slice(&1u32.to_be_bytes());
            tiff.extend_from_slice(&40u32.to_be_bytes());
        }
        tiff.extend_from_slice(&0u32.to_be_bytes());
        tiff.extend_from_slice(b"GPS 52.5200 N 13.4050 E");
        tiff
    }

    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut s = vec![0xFF, marker];
        s.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        s.extend_from_slice(payload);
        s
    }

    /// A JPEG skeleton: SOI, the given segments, a frame header, one scan
    /// with stuffed bytes and a restart marker, EOI, and a trailer.
    fn jpeg(segments: &[Vec<u8>], trailer: &[u8]) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8];
        for s in segments {
            j.extend_from_slice(s);
        }
        j.extend_from_slice(&segment(0xC0, &[8, 0, 30, 0, 40, 1, 1, 0x11, 0]));
        j.extend_from_slice(&segment(0xDA, &[1, 1, 0, 0, 63, 0]));
        j.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56]);
        j.extend_from_slice(&[0xFF, 0xD9]);
        j.extend_from_slice(trailer);
        j
    }

    #[test]
    fn jpeg_loses_gps_xmp_and_trailer_but_keeps_orientation_and_icc() {
        let mut exif = EXIF_HEADER.to_vec();
        exif.extend_from_slice(&exif_tiff(6, true));
        let mut icc = ICC_HEADER.to_vec();
        icc.extend_from_slice(&[1, 1, 9, 9, 9]);
        let source = jpeg(
            &[
                segment(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"),
                segment(0xE1, &exif),
                segment(
                    0xE1,
                    b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>GPS</x:xmpmeta>",
                ),
                segment(0xE2, &icc),
                segment(0xE2, b"MPF\0index"),
                segment(0xED, b"Photoshop 3.0\0IPTC"),
                segment(0xFE, b"a comment"),
            ],
            b"\xFF\xD8motion photo video with GPS",
        );
        assert_eq!(orientation("image/jpeg", &source), 6);
        assert_eq!(display_size("image/jpeg", &source), Some((30, 40)));

        let out = strip("image/jpeg", &source).expect("something removed");
        assert!(!out.windows(3).any(|w| w == b"GPS"));
        assert!(!out.windows(3).any(|w| w == b"MPF"));
        assert!(out.windows(2).any(|w| w == [0xFF, 0xDA]));
        assert!(out.ends_with(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56, 0xFF, 0xD9]));
        assert!(out.windows(ICC_HEADER.len()).any(|w| w == ICC_HEADER));
        assert!(out.windows(5).any(|w| w == b"JFIF\0"));
        assert_eq!(orientation("image/jpeg", &out), 6);
        // Already clean: nothing to rewrite.
        assert_eq!(strip("image/jpeg", &out), None);
    }

    #[test]
    fn jpeg_with_upright_exif_keeps_no_exif_at_all() {
        let mut exif = EXIF_HEADER.to_vec();
        exif.extend_from_slice(&exif_tiff(1, true));
        let out = strip("image/jpeg", &jpeg(&[segment(0xE1, &exif)], b"")).unwrap();
        assert!(!out.windows(4).any(|w| w == b"Exif"));
        assert_eq!(orientation("image/jpeg", &out), 1);
    }

    #[test]
    fn broken_files_are_left_alone() {
        let mut exif = EXIF_HEADER.to_vec();
        exif.extend_from_slice(&exif_tiff(6, true));
        let whole = jpeg(&[segment(0xE1, &exif)], b"");
        for cut in [3, 10, whole.len() - 3] {
            assert_eq!(strip("image/jpeg", &whole[..cut]), None, "cut at {cut}");
        }
        assert_eq!(strip("image/png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), None);
        assert_eq!(strip("image/webp", b"RIFF\x04\0\0\0WEBP"), None);
        assert_eq!(strip("image/gif", b"GIF89a"), None);
        assert_eq!(orientation("image/jpeg", b"\xFF\xD8\xFF"), 1);
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = (data.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(kind);
        c.extend_from_slice(data);
        let crc = crc32(&c[4..]);
        c.extend_from_slice(&crc.to_be_bytes());
        c
    }

    #[test]
    fn png_loses_text_chunks_and_keeps_orientation() {
        let mut png = PNG_SIGNATURE.to_vec();
        let mut ihdr = 30u32.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&40u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend(png_chunk(b"IHDR", &ihdr));
        png.extend(png_chunk(b"iCCP", b"icc"));
        png.extend(png_chunk(b"eXIf", &exif_tiff(8, true)));
        png.extend(png_chunk(b"iTXt", b"XML:com.adobe.xmp\0GPS"));
        png.extend(png_chunk(b"tEXt", b"Comment\0GPS"));
        png.extend(png_chunk(b"IDAT", b"pixels"));
        png.extend(png_chunk(b"IEND", b""));
        assert_eq!(display_size("image/png", &png), Some((40, 30)));
        let out = strip("image/png", &png).unwrap();
        assert!(!out.windows(3).any(|w| w == b"GPS"));
        assert!(out.windows(4).any(|w| w == b"iCCP"));
        assert_eq!(orientation("image/png", &out), 8);
        // Every chunk still has a valid CRC.
        for chunk in png_chunks(&out).unwrap() {
            let crc = u32::from_be_bytes(chunk.raw[chunk.raw.len() - 4..].try_into().unwrap());
            assert_eq!(crc, crc32(&chunk.raw[4..chunk.raw.len() - 4]));
        }
        assert!(!animated("image/png", &out));
    }

    fn webp_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = kind.to_vec();
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(data);
        if data.len() % 2 == 1 {
            c.push(0);
        }
        c
    }

    fn riff(body: &[u8]) -> Vec<u8> {
        let mut w = b"RIFF".to_vec();
        w.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        w.extend_from_slice(b"WEBP");
        w.extend_from_slice(body);
        w
    }

    #[test]
    fn webp_loses_exif_and_xmp_and_fixes_flags_and_size() {
        let mut body = webp_chunk(b"VP8X", &[0x20 | 0x08 | 0x04, 0, 0, 0, 29, 0, 0, 39, 0, 0]);
        body.extend(webp_chunk(b"ICCP", b"icc"));
        body.extend(webp_chunk(b"VP8 ", b"frame"));
        body.extend(webp_chunk(b"EXIF", &exif_tiff(1, true)));
        body.extend(webp_chunk(b"XMP ", b"<GPS/>"));
        let source = riff(&body);
        let out = strip("image/webp", &source).unwrap();
        assert!(!out.windows(3).any(|w| w == b"GPS"));
        let chunks = webp_chunks(&out).unwrap();
        let kinds: Vec<_> = chunks.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [*b"VP8X", *b"ICCP", *b"VP8 "]);
        assert_eq!(chunks[0].data[0], 0x20);
        assert_eq!(
            u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize,
            out.len() - 8
        );
        assert!(!animated("image/webp", &out));
    }

    #[test]
    fn gif_frames_are_counted() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0]);
        let frame = [0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x4C, 0x01, 0];
        gif.extend_from_slice(&[0x21, 0xF9, 4, 0, 0, 0, 0, 0]);
        gif.extend_from_slice(&frame);
        assert!(!animated("image/gif", &gif));
        gif.extend_from_slice(&frame);
        assert!(animated("image/gif", &gif));
        assert_eq!(display_size("image/gif", &gif), Some((1, 1)));
    }
}
