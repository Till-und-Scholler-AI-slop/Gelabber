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
//! - PNG: every chunk but the ones that draw the picture (an allowlist),
//!   so `eXIf` (except the orientation), text chunks (where XMP lives),
//!   `tIME`, C2PA manifests (`caBX`) and unknown ones go.
//! - WebP: every chunk but `VP8X`, `VP8 `, `VP8L`, `ALPH`, `ANIM`, `ANMF`
//!   and `ICCP` (an allowlist); `EXIF` keeps only the orientation.
//! - GIF: XMP application extensions. It has no EXIF.
//! - HEIC/HEIF (stored, never decoded): the `Exif` item and XMP (`mime`
//!   items of `application/rdf+xml`) are overwritten with zeros in place,
//!   so no offset moves. Its orientation lives in `irot`/`imir`.
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
    let stripped = parsed_strip(content_type, bytes);
    if stripped.is_none() && strips(content_type) {
        tracing::debug!(content_type, "picture metadata not understood, kept as is");
    }
    let stripped = stripped?;
    (stripped != bytes).then_some(stripped)
}

/// The types [`strip`] knows how to clean.
pub fn strips(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg" | "image/png" | "image/webp" | "image/gif" | "image/heic" | "image/heif"
    )
}

fn parsed_strip(content_type: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    Some(match content_type {
        "image/jpeg" => strip_jpeg(bytes)?,
        "image/png" => strip_png(bytes)?,
        "image/webp" => strip_webp(bytes)?,
        "image/gif" => strip_gif(bytes)?,
        "image/heic" | "image/heif" => strip_heif(bytes)?,
        _ => return None,
    })
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
            kind if PNG_KEPT.contains(kind) => out.extend_from_slice(chunk.raw),
            _ => {}
        }
    }
    Some(out)
}

/// PNG chunks that draw the picture; every other one goes. `eXIf` is
/// handled on its own.
const PNG_KEPT: &[[u8; 4]] = &[
    *b"IHDR", *b"PLTE", *b"IDAT", *b"IEND", *b"tRNS", *b"gAMA", *b"cHRM", *b"sRGB", *b"iCCP",
    *b"cICP", *b"sBIT", *b"bKGD", *b"pHYs", *b"acTL", *b"fcTL", *b"fdAT",
];

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
    // VP8X is 10 bytes by the spec; its flags are written below.
    let first = chunks.first()?;
    if &first.kind != b"VP8X" || first.data.len() < 10 {
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
            kind if WEBP_KEPT.contains(kind) => {
                body.extend_from_slice(chunk.raw);
                if chunk.raw.len() < 8 + chunk.data.len() + (chunk.data.len() & 1) {
                    // A last chunk cut short of its padding.
                    body.push(0);
                }
            }
            _ => {}
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

/// WebP chunks that draw the picture; every other one goes (`XMP `,
/// `C2PA`, unknown ones). `EXIF` is handled on its own.
const WEBP_KEPT: &[[u8; 4]] = &[
    *b"VP8X", *b"VP8 ", *b"VP8L", *b"ALPH", *b"ANIM", *b"ANMF", *b"ICCP",
];

// --- GIF -----------------------------------------------------------------

/// The GIF without XMP application extensions ("XMP DataXMP"); `None`
/// when it cannot be walked to its trailer.
fn strip_gif(bytes: &[u8]) -> Option<Vec<u8>> {
    if !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let flags = *bytes.get(10)?;
    let mut at = 13;
    if flags & 0x80 != 0 {
        at += 3 << ((flags & 0x07) + 1);
    }
    let blocks_end = |mut at: usize| -> Option<usize> {
        loop {
            let len = usize::from(*bytes.get(at)?);
            at += 1 + len;
            if len == 0 {
                return (at <= bytes.len()).then_some(at);
            }
        }
    };
    let mut out = bytes.get(..at)?.to_vec();
    loop {
        match *bytes.get(at)? {
            0x21 => {
                let end = blocks_end(at + 2)?;
                let xmp = bytes.get(at + 1) == Some(&0xFF)
                    && bytes.get(at + 2) == Some(&11)
                    && bytes.get(at + 3..at + 14) == Some(b"XMP DataXMP");
                if !xmp {
                    out.extend_from_slice(&bytes[at..end]);
                }
                at = end;
            }
            0x2C => {
                let local = *bytes.get(at + 9)?;
                let mut end = at + 10;
                if local & 0x80 != 0 {
                    end += 3 << ((local & 0x07) + 1);
                }
                // LZW minimum code size, then the data sub-blocks.
                let end = blocks_end(end + 1)?;
                out.extend_from_slice(&bytes[at..end]);
                at = end;
            }
            0x3B => {
                out.push(0x3B);
                return Some(out);
            }
            _ => return None,
        }
    }
}

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

// --- HEIF ----------------------------------------------------------------

/// A box: its type, its body, and where its body starts in the file.
struct IsoBox<'a> {
    kind: [u8; 4],
    body: &'a [u8],
    start: usize,
}

/// The boxes in `bytes[from..to]`; `None` when one runs past the end.
fn iso_boxes(bytes: &[u8], from: usize, to: usize) -> Option<Vec<IsoBox<'_>>> {
    let mut at = from;
    let mut boxes = Vec::new();
    while at + 8 <= to {
        let size = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind: [u8; 4] = bytes.get(at + 4..at + 8)?.try_into().ok()?;
        let (header, size) = match size {
            0 => (8, to - at),
            1 => {
                let large = u64::from_be_bytes(bytes.get(at + 8..at + 16)?.try_into().ok()?);
                (16, usize::try_from(large).ok()?)
            }
            n => (8, n),
        };
        let end = at.checked_add(size)?;
        if size < header || end > to {
            return None;
        }
        boxes.push(IsoBox {
            kind,
            body: &bytes[at + header..end],
            start: at + header,
        });
        at = end;
    }
    Some(boxes)
}

/// A big-endian unsigned number of `n` bytes (0, 2, 4 or 8) at `*at`.
fn be(bytes: &[u8], at: &mut usize, n: usize) -> Option<u64> {
    #[cfg(test)]
    tests::READS.with(|reads| reads.set(reads.get() + 1));
    let mut value = 0u64;
    for byte in bytes.get(*at..at.checked_add(n)?)? {
        value = (value << 8) | u64::from(*byte);
    }
    *at += n;
    Some(value)
}

/// The ids of the `Exif` item and of XMP `mime` items, from `iinf`.
fn heif_metadata_items(iinf: &[u8]) -> Option<Vec<u32>> {
    let version = *iinf.first()?;
    let mut at = 4;
    let _count = be(iinf, &mut at, if version == 0 { 2 } else { 4 })?;
    let mut ids = Vec::new();
    for infe in iso_boxes(iinf, at, iinf.len())? {
        if &infe.kind != b"infe" {
            continue;
        }
        let b = infe.body;
        let version = *b.first()?;
        if version < 2 {
            continue;
        }
        let mut at = 4;
        let id = be(b, &mut at, if version == 2 { 2 } else { 4 })? as u32;
        at += 2; // protection index
        let kind = b.get(at..at + 4)?;
        at += 4;
        let xmp = kind == b"mime" && {
            let rest = b.get(at..)?;
            // item_name, then content_type, both NUL-terminated.
            let mut parts = rest.split(|c| *c == 0);
            let _name = parts.next();
            parts.next() == Some(b"application/rdf+xml".as_slice())
        };
        if kind == b"Exif" || xmp {
            ids.push(id);
        }
    }
    Some(ids)
}

/// More extents than any real metadata item has.
const HEIF_EXTENTS_MAX: usize = 1024;

/// The file ranges of `items`, from `iloc` (offsets into this file only).
fn heif_extents(iloc: &[u8], items: &[u32]) -> Option<Vec<(usize, usize)>> {
    let version = *iloc.first()?;
    if version > 2 {
        return None;
    }
    let mut at = 4;
    let sizes = be(iloc, &mut at, 1)? as usize;
    let more = be(iloc, &mut at, 1)? as usize;
    let (offset_size, length_size) = (sizes >> 4, sizes & 0x0F);
    let base_size = more >> 4;
    let index_size = if version == 0 { 0 } else { more & 0x0F };
    // The spec allows only these field widths; anything else is not a
    // file we understand.
    if ![offset_size, length_size, base_size, index_size]
        .iter()
        .all(|n| matches!(n, 0 | 4 | 8))
    {
        return None;
    }
    let entry = index_size + offset_size + length_size;
    let count = be(iloc, &mut at, if version < 2 { 2 } else { 4 })?;
    let mut ranges = Vec::new();
    for _ in 0..count {
        let id = be(iloc, &mut at, if version < 2 { 2 } else { 4 })? as u32;
        let method = if version == 0 {
            0
        } else {
            be(iloc, &mut at, 2)? & 0x0F
        };
        let reference = be(iloc, &mut at, 2)?;
        let base = be(iloc, &mut at, base_size)?;
        let extents = be(iloc, &mut at, 2)? as usize;
        if !items.contains(&id) {
            // Skipped without a loop: zero-width fields would otherwise
            // make 65535 empty reads per item cost CPU, not bytes.
            at = at.checked_add(extents.checked_mul(entry)?)?;
            if at > iloc.len() {
                return None;
            }
            continue;
        }
        if ranges.len() + extents > HEIF_EXTENTS_MAX {
            return None;
        }
        for _ in 0..extents {
            be(iloc, &mut at, index_size)?;
            let offset = be(iloc, &mut at, offset_size)?;
            let length = be(iloc, &mut at, length_size)?;
            // Only data in this file; a length of 0 ("to the end") is not
            // something a metadata item uses.
            if method != 0 || reference != 0 || length == 0 {
                return None;
            }
            let start = usize::try_from(base.checked_add(offset)?).ok()?;
            let len = usize::try_from(length).ok()?;
            ranges.push((start, start.checked_add(len)?));
        }
    }
    Some(ranges)
}

/// The HEIF with its `Exif` and XMP items zeroed in place.
fn strip_heif(bytes: &[u8]) -> Option<Vec<u8>> {
    let top = iso_boxes(bytes, 0, bytes.len())?;
    if top.first().map(|b| &b.kind) != Some(b"ftyp") {
        return None;
    }
    let meta = top.iter().find(|b| &b.kind == b"meta")?;
    // A full box: version and flags first.
    let children = iso_boxes(bytes, meta.start + 4, meta.start + meta.body.len())?;
    let iinf = children.iter().find(|b| &b.kind == b"iinf")?;
    let iloc = children.iter().find(|b| &b.kind == b"iloc")?;
    let items = heif_metadata_items(iinf.body)?;
    if items.is_empty() {
        return Some(bytes.to_vec());
    }
    let mut out = bytes.to_vec();
    for (start, end) in heif_extents(iloc.body, &items)? {
        out.get_mut(start..end)?.fill(0);
    }
    Some(out)
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
        // A VP8X shorter than its 10 bytes, alone or before a frame.
        assert_eq!(strip("image/webp", b"RIFF\x0c\0\0\0WEBPVP8X\0\0\0\0"), None);
        let mut short = webp_chunk(b"VP8X", b"");
        short.extend(webp_chunk(b"VP8L", b"frame"));
        assert_eq!(strip("image/webp", &riff(&short)), None);
        assert_eq!(strip("image/heic", b"\0\0\0\x18ftypheic\0\0"), None);
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
        png.extend(png_chunk(b"caBX", b"c2pa GPS"));
        png.extend(png_chunk(b"zzZz", b"unknown GPS"));
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
        body.extend(webp_chunk(b"C2PA", b"GPS"));
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
    fn gif_loses_xmp_and_keeps_its_frames() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0]);
        let netscape = [
            0x21, 0xFF, 11, b'N', b'E', b'T', b'S', b'C', b'A', b'P', b'E', b'2', b'.', b'0', 3, 1,
            0, 0, 0,
        ];
        gif.extend_from_slice(&netscape);
        gif.extend_from_slice(&[0x21, 0xFF, 11]);
        gif.extend_from_slice(b"XMP DataXMP");
        gif.extend_from_slice(&[4, b'G', b'P', b'S', b'!', 0]);
        let frame = [0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x4C, 0x01, 0];
        gif.extend_from_slice(&frame);
        gif.extend_from_slice(&frame);
        gif.push(0x3B);
        let out = strip("image/gif", &gif).unwrap();
        assert!(!out.windows(3).any(|w| w == b"GPS"));
        assert!(out.windows(8).any(|w| w == b"NETSCAPE"));
        assert!(animated("image/gif", &out));
        assert_eq!(out.last(), Some(&0x3B));
        // Nothing to remove: left alone.
        assert_eq!(strip("image/gif", &out), None);
        // Cut short: left alone.
        assert_eq!(strip("image/gif", &gif[..gif.len() - 4]), None);
    }

    fn iso(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    /// A HEIC as a phone writes it: a picture item, an `Exif` item and an
    /// XMP item, all in `mdat`, located by `iloc` version 1.
    fn heic() -> (Vec<u8>, std::ops::Range<usize>) {
        let infe = |id: u16, kind: &[u8; 4], extra: &[u8]| {
            let mut b = vec![2, 0, 0, 0];
            b.extend_from_slice(&id.to_be_bytes());
            b.extend_from_slice(&[0, 0]);
            b.extend_from_slice(kind);
            b.extend_from_slice(extra);
            iso(b"infe", &b)
        };
        let mut iinf = vec![0, 0, 0, 0, 0, 3];
        iinf.extend(infe(1, b"hvc1", b"\0"));
        iinf.extend(infe(2, b"Exif", b"\0"));
        iinf.extend(infe(3, b"mime", b"\0application/rdf+xml\0"));
        let pixels = b"PIXELS-PIXELS";
        let exif = b"\0\0\0\x06Exif\0\0GPS 52.52 N";
        let xmp = b"<x:xmpmeta>GPS</x:xmpmeta>";
        let ftyp = iso(b"ftyp", b"heic\0\0\0\0mif1heic");
        // iloc v1: offset 4, length 4, base 0, index 0; 3 items.
        let iloc_len = 4 + 2 + 2 + 3 * (2 + 2 + 2 + 2 + 4 + 4) + 8;
        let mut meta_body = vec![0, 0, 0, 0];
        meta_body.extend(iso(b"hdlr", &[0; 24]));
        meta_body.extend(iso(b"iinf", &iinf));
        let meta_len = 8 + meta_body.len() + iloc_len;
        let mdat_start = ftyp.len() + meta_len + 8;
        let mut iloc = vec![1, 0, 0, 0, 0x44, 0x00, 0, 3];
        let mut at = mdat_start;
        for (id, len) in [(1u16, pixels.len()), (2, exif.len()), (3, xmp.len())] {
            iloc.extend_from_slice(&id.to_be_bytes());
            iloc.extend_from_slice(&[0, 0, 0, 0, 0, 1]);
            iloc.extend_from_slice(&(at as u32).to_be_bytes());
            iloc.extend_from_slice(&(len as u32).to_be_bytes());
            at += len;
        }
        meta_body.extend(iso(b"iloc", &iloc));
        let mut file = ftyp;
        file.extend(iso(b"meta", &meta_body));
        assert_eq!(file.len() + 8, mdat_start);
        let mut mdat = pixels.to_vec();
        mdat.extend_from_slice(exif);
        mdat.extend_from_slice(xmp);
        file.extend(iso(b"mdat", &mdat));
        (file, mdat_start..mdat_start + pixels.len())
    }

    /// An `iloc` v1 body: field widths, then `entries` items of
    /// (id, method 0, data reference 0, extent count).
    fn iloc_v1(widths: [u8; 2], entries: &[(u16, u16)]) -> Vec<u8> {
        let mut body = vec![1, 0, 0, 0, widths[0], widths[1]];
        body.extend_from_slice(&(entries.len() as u16).to_be_bytes());
        for (id, extents) in entries {
            body.extend_from_slice(&id.to_be_bytes());
            body.extend_from_slice(&[0, 0, 0, 0]);
            body.extend_from_slice(&extents.to_be_bytes());
        }
        body
    }

    thread_local! {
        /// Fields `be` read on this thread: the cost of a parse, counted.
        pub(super) static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn heif_iloc_with_empty_fields_costs_no_cpu() {
        // Zero-width fields and 0xFFFF extents per item: once took
        // seconds per few KB (three empty reads per extent), now skipped
        // arithmetically: five fields per item, whatever its extents.
        let others: Vec<(u16, u16)> = (2..1_002).map(|id| (id, 0xFFFF)).collect();
        READS.with(|reads| reads.set(0));
        assert_eq!(heif_extents(&iloc_v1([0, 0], &others), &[1]), Some(vec![]));
        let reads = READS.with(std::cell::Cell::get);
        assert!(reads <= 5 * others.len() + 3, "{reads} reads");
        // The metadata item itself with that many extents is refused.
        assert_eq!(heif_extents(&iloc_v1([0, 0], &[(1, 0xFFFF)]), &[1]), None);
        // Field widths the spec does not allow.
        assert_eq!(heif_extents(&iloc_v1([0x11, 0], &[(1, 1)]), &[1]), None);
        assert_eq!(heif_extents(&iloc_v1([0x44, 0x03], &[(1, 1)]), &[1]), None);
    }

    #[test]
    fn heic_loses_exif_and_xmp_in_place() {
        let (source, pixels) = heic();
        assert!(super::super::sniff::matches("image/heic", &source));
        let out = strip("image/heic", &source).unwrap();
        assert_eq!(out.len(), source.len());
        assert!(!out.windows(3).any(|w| w == b"GPS"));
        assert_eq!(out[pixels.clone()], source[pixels]);
        assert_eq!(strip("image/heic", &out), None);
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
