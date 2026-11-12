//! Which files a message takes (messages/rules.ts `validateAttachment`,
//! `validateAttachmentSet`, `inferContentType`; Composer.tsx `addFiles`;
//! messages/clipboard.ts for pasted pictures), mirroring the API's checks.

use gelabber_client::OutgoingFile;

/// `SIZE_MAX`, `FILENAME_MAX`, `ATTACHMENTS_MAX`, `MESSAGE_SIZE_MAX`.
const SIZE_MAX: usize = 25 * 1024 * 1024;
const FILENAME_MAX: usize = 255;
const ATTACHMENTS_MAX: usize = 10;
const MESSAGE_SIZE_MAX: usize = 100 * 1024 * 1024;

const ALLOWED_TYPES: [&str; 23] = [
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "application/pdf",
    "text/plain",
    "text/markdown",
    "text/csv",
    "application/json",
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/x-7z-compressed",
    "audio/mpeg",
    "audio/wav",
    "audio/ogg",
    "audio/flac",
    "audio/mp4",
    "audio/aac",
    "audio/webm",
    "video/mp4",
    "video/webm",
    "video/quicktime",
];

/// Source code and other text: stored as text/plain, the name keeps the
/// extension. HTML and SVG are deliberately absent.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "log", "ini", "cfg", "conf", "toml", "yaml", "yml", "env", "rs", "py", "js", "mjs",
    "cjs", "ts", "tsx", "jsx", "c", "h", "cc", "cpp", "hpp", "cs", "java", "kt", "go", "rb", "php",
    "swift", "lua", "sh", "bash", "zsh", "fish", "ps1", "sql", "css", "scss", "diff", "patch",
    "nix", "zig", "hs", "ex", "exs", "dart", "r", "pl", "vue", "svelte", "gradle", "lock",
];

/// `EXT_TYPE`.
fn ext_type(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "weba" => "audio/webm",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        other if TEXT_EXTENSIONS.contains(&other) => "text/plain",
        _ => return None,
    })
}

/// What a browser would say of a file it may not send: the system's type
/// for common names, so the note says "nicht erlaubt", not "fehlt".
fn system_type(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "exe" => "application/x-msdownload",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "odt" => "application/vnd.oasis.opendocument.text",
        "iso" => "application/x-iso9660-image",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        _ => "",
    }
}

/// `inferContentType`: a known `given` type, else the extension's; a file
/// from the disk carries no type, so the name decides.
pub fn infer_content_type(name: &str, given: &str) -> String {
    let given = if given.is_empty() {
        system_type(name)
    } else {
        given
    };
    let raw = given.trim().to_lowercase();
    let raw = raw.split(';').next().unwrap_or_default().trim();
    if ALLOWED_TYPES.contains(&raw) {
        return raw.to_owned();
    }
    let ext = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    match ext_type(&ext).filter(|_| name.contains('.')) {
        Some(t) => t.to_owned(),
        None => raw.to_owned(),
    }
}

/// `validateAttachment`: the field and code the API would give, or `None`.
pub fn validate_attachment(file: &OutgoingFile) -> Option<(&'static str, &'static str)> {
    let name = file
        .filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    if name.is_empty() {
        return Some(("filename", "required"));
    }
    if name.chars().count() > FILENAME_MAX {
        return Some(("filename", "too_long"));
    }
    let content_type = infer_content_type(&file.filename, &file.content_type);
    if content_type.is_empty() {
        return Some(("content_type", "required"));
    }
    if !ALLOWED_TYPES.contains(&content_type.as_str()) {
        return Some(("content_type", "invalid"));
    }
    let size = file.data.len();
    if size == 0 {
        return Some(("size", "required"));
    }
    if size > SIZE_MAX {
        return Some(("size", "too_long"));
    }
    None
}

/// `validateAttachmentSet`: one message's files together.
pub fn validate_attachment_set(sizes: &[usize]) -> Option<(&'static str, &'static str)> {
    if sizes.len() > ATTACHMENTS_MAX {
        return Some(("attachment_ids", "too_long"));
    }
    if sizes.iter().sum::<usize>() > MESSAGE_SIZE_MAX {
        return Some(("attachment_ids", "too_large"));
    }
    None
}

/// `addFiles`: adds `next` to `files`, leaving out invalid ones and what
/// would go over the message's limits; the note names every file left
/// out ("<name>: <reason>", "Nicht angehängt: a, b. <reason>").
pub fn add_files(files: &mut Vec<OutgoingFile>, next: Vec<OutgoingFile>) -> Option<String> {
    let mut notes = Vec::new();
    let mut over: Vec<(String, Vec<String>)> = Vec::new();
    for mut candidate in next {
        if let Some((field, code)) = validate_attachment(&candidate) {
            notes.push(format!(
                "{}: {}",
                candidate.filename,
                crate::rules::field_message(field, code)
            ));
            continue;
        }
        // A smaller file further on may still fit under the size limit.
        let mut sizes: Vec<usize> = files.iter().map(|f| f.data.len()).collect();
        sizes.push(candidate.data.len());
        if let Some((field, code)) = validate_attachment_set(&sizes) {
            let reason = crate::rules::field_message(field, code);
            match over.iter_mut().find(|(r, _)| *r == reason) {
                Some((_, names)) => names.push(candidate.filename),
                None => over.push((reason, vec![candidate.filename])),
            }
            continue;
        }
        candidate.content_type = infer_content_type(&candidate.filename, &candidate.content_type);
        files.push(candidate);
    }
    for (reason, names) in over {
        notes.push(gelabber_i18n::chat::composer_not_attached(
            &names.join(", "),
            &reason,
        ));
    }
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// Where an attached file comes from.
pub enum Source {
    /// A file on the disk (chosen or dropped).
    Path(std::path::PathBuf),
    /// A picture pasted into the draft.
    Pasted(std::sync::Arc<egui::ColorImage>),
}

/// The files of [`load`] once they are read, and the notes for those left
/// out before reading.
pub type Loaded = (Vec<OutgoingFile>, Vec<String>);
pub type Loading = std::sync::Arc<parking_lot::Mutex<Option<Loaded>>>;

/// Reads `sources` on a worker thread; a file over [`SIZE_MAX`] by its size
/// on disk is named, not read.
pub fn load(
    ctx: &egui::Context,
    sources: Vec<Source>,
    now: chrono::DateTime<chrono::Utc>,
) -> Loading {
    let slot = Loading::default();
    let out = slot.clone();
    let ctx = ctx.clone();
    let work = move || {
        let mut files = Vec::new();
        let mut notes = Vec::new();
        for source in sources {
            match source {
                Source::Path(path) => {
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    match std::fs::metadata(&path) {
                        Ok(meta) if meta.len() > SIZE_MAX as u64 => notes.push(format!(
                            "{filename}: {}",
                            crate::rules::field_message("size", "too_long")
                        )),
                        Ok(meta) if meta.is_file() => {
                            if let Ok(data) = std::fs::read(&path) {
                                files.push(OutgoingFile {
                                    content_type: infer_content_type(&filename, ""),
                                    filename,
                                    data: std::sync::Arc::new(data),
                                });
                            }
                        }
                        _ => {}
                    }
                }
                Source::Pasted(image) => files.extend(pasted_image(&image, now)),
            }
        }
        *out.lock() = Some((files, notes));
        ctx.request_repaint();
    };
    if let Err(e) = std::thread::Builder::new()
        .name("gelabber-attach".into())
        .spawn(work)
    {
        tracing::warn!("attach: no thread: {e}");
        *slot.lock() = Some((Vec::new(), Vec::new()));
    }
    slot
}

/// A pasted picture as a PNG file named like the web's
/// "Bild-20261105-143000.png".
pub fn pasted_image(
    image: &egui::ColorImage,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<OutgoingFile> {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|p| p.to_srgba_unmultiplied())
        .collect();
    let buffer = image::RgbaImage::from_raw(w as u32, h as u32, rgba)?;
    let mut png = Vec::new();
    buffer
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(OutgoingFile {
        filename: format!(
            "{}.png",
            gelabber_i18n::chat::composer_pasted_image_name(
                &now.format("%Y%m%d-%H%M%S").to_string()
            )
        ),
        content_type: "image/png".into(),
        data: std::sync::Arc::new(png),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: usize) -> OutgoingFile {
        OutgoingFile {
            filename: name.into(),
            content_type: String::new(),
            data: std::sync::Arc::new(vec![1; size]),
        }
    }

    #[test]
    fn types_come_from_the_name() {
        assert_eq!(infer_content_type("main.rs", ""), "text/plain");
        assert_eq!(infer_content_type("Foto.JPG", ""), "image/jpeg");
        assert_eq!(infer_content_type("a.mov", ""), "video/quicktime");
        assert_eq!(infer_content_type("seite.html", ""), "text/html");
        assert_eq!(infer_content_type("notiz.xyz", ""), "");
        assert_eq!(infer_content_type("x", "image/png"), "image/png");
    }

    #[test]
    fn a_file_the_api_refuses_is_named_with_its_reason() {
        let mut files = Vec::new();
        let note = add_files(
            &mut files,
            vec![
                file("seite.html", 10),
                file("leer.txt", 0),
                file("ok.txt", 10),
            ],
        );
        assert_eq!(
            note.as_deref(),
            Some("seite.html: Dieser Dateityp ist nicht erlaubt. leer.txt: Datei fehlt.")
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content_type, "text/plain");
        let mut files = Vec::new();
        let note = add_files(&mut files, vec![file("gross.zip", SIZE_MAX + 1)]);
        assert_eq!(
            note.as_deref(),
            Some("gross.zip: Die Datei ist zu groß (max. 25 MB).")
        );
    }

    #[test]
    fn what_goes_over_the_messages_limits_is_left_out_together() {
        let mut files: Vec<OutgoingFile> = (0..9).map(|i| file(&format!("{i}.txt"), 1)).collect();
        let note = add_files(
            &mut files,
            vec![file("a.txt", 1), file("b.txt", 1), file("c.txt", 1)],
        );
        assert_eq!(
            note.as_deref(),
            Some("Nicht angehängt: b.txt, c.txt. Höchstens 10 Dateien pro Nachricht.")
        );
        assert_eq!(files.len(), 10);
        // Over the total: a smaller one after a big one still fits.
        let mut files = vec![file("a.zip", 20 * 1024 * 1024); 4];
        let note = add_files(
            &mut files,
            vec![file("b.zip", 25 * 1024 * 1024), file("c.zip", 1024)],
        );
        assert_eq!(
            note.as_deref(),
            Some("Nicht angehängt: b.zip. Alle Dateien zusammen sind zu groß (max. 100 MB).")
        );
        assert_eq!(files.len(), 5);
    }

    #[test]
    fn a_pasted_picture_becomes_a_named_png() {
        let image = egui::ColorImage::filled([2, 2], egui::Color32::RED);
        let now = "2026-11-05T14:30:00Z".parse().unwrap();
        let file = pasted_image(&image, now).expect("png");
        assert_eq!(file.filename, "Bild-20261105-143000.png");
        assert_eq!(validate_attachment(&file), None);
        assert_eq!(crate::images::decode(&file.data).unwrap().size, [2, 2]);
    }
}
