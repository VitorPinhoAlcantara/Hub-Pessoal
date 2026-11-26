use std::path::{Path, PathBuf};

use super::MessageKind;

pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_PICTURE_BYTES: u64 = 5 * 1024 * 1024;

const DANGEROUS_EXTENSIONS: &[&str] = &[
    "exe",
    "com",
    "bat",
    "cmd",
    "msi",
    "msp",
    "scr",
    "pif",
    "lnk",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "cpl",
    "reg",
    "jar",
    "dll",
    "sh",
    "appx",
    "msix",
    "gadget",
    "application",
    "url",
    "inf",
    "scf",
];

const RESERVED_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutKind {
    Image,
    Video,
    Audio,
    Document,
}

pub fn sanitize_id(id: &str) -> String {
    let clean: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if clean.is_empty() {
        "midia".to_string()
    } else {
        clean
    }
}

pub fn safe_file_name(original: &str, fallback_extension: &str) -> String {
    let last = original.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    let (stem, extension) = match cleaned.rfind('.') {
        Some(index) if index > 0 => (&cleaned[..index], &cleaned[index + 1..]),
        _ => (cleaned.as_str(), ""),
    };
    let mut stem: String = stem.trim().trim_end_matches('.').chars().take(80).collect();
    let first_segment = stem.split('.').next().unwrap_or("").to_lowercase();
    if RESERVED_NAMES.contains(&first_segment.as_str()) {
        stem.insert(0, '_');
    }
    if stem.is_empty() {
        stem = "arquivo".to_string();
    }
    let extension: String = extension
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(10)
        .collect::<String>()
        .to_lowercase();
    let extension = if DANGEROUS_EXTENSIONS.contains(&extension.as_str()) {
        "bin".to_string()
    } else if extension.is_empty() {
        fallback_extension.to_string()
    } else {
        extension
    };
    if extension.is_empty() {
        stem
    } else {
        format!("{stem}.{extension}")
    }
}

pub fn extension_for_mime(mime: &str) -> &'static str {
    match mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase()
        .as_str()
    {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "video/3gpp" => "3gp",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/aac" | "audio/x-m4a" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/amr" => "amr",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "application/zip" => "zip",
        _ => "",
    }
}

pub fn mime_for_extension(extension: &str) -> &'static str {
    match extension.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "mp4" | "m4v" => "video/mp4",
        "3gp" => "video/3gpp",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "opus" | "oga" => "audio/ogg; codecs=opus",
        "wav" => "audio/wav",
        "amr" => "audio/amr",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

pub fn classify_path(path: &Path) -> (OutKind, String) {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = mime_for_extension(&extension).to_string();
    let kind = match extension.as_str() {
        "jpg" | "jpeg" | "png" | "webp" => OutKind::Image,
        "mp4" | "m4v" | "3gp" => OutKind::Video,
        "mp3" | "m4a" | "aac" | "ogg" | "opus" | "oga" | "wav" | "amr" | "flac" => OutKind::Audio,
        _ => OutKind::Document,
    };
    (kind, mime)
}

pub fn media_file_name(id: &str, kind: MessageKind, mime: &str, original_name: &str) -> String {
    let id = sanitize_id(id);
    let fallback = extension_for_mime(mime);
    if kind == MessageKind::Document {
        let name = safe_file_name(original_name, fallback);
        format!("{id}_{name}")
    } else {
        let extension = match fallback {
            "" => "bin",
            other => other,
        };
        format!("{id}.{extension}")
    }
}

pub fn media_path(
    dir: &Path,
    id: &str,
    kind: MessageKind,
    mime: &str,
    original_name: &str,
) -> PathBuf {
    dir.join(media_file_name(id, kind, mime, original_name))
}

pub fn photo_file_name(jid: &str) -> String {
    let clean: String = jid
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(80)
        .collect();
    format!("{clean}.jpg")
}

pub fn is_trusted_picture_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let host = authority.split(':').next().unwrap_or("").to_lowercase();
    host.ends_with(".whatsapp.net") || host.ends_with(".fbcdn.net")
}

pub fn is_inside(dir: &Path, path: &Path) -> bool {
    path.starts_with(dir)
        && path
            .components()
            .all(|c| !matches!(c, std::path::Component::ParentDir))
}
