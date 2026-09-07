use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_hidden: bool,
    pub size: u64,
    pub modified: SystemTime,
    pub icon_name: String,
    pub mime_approx: String,
}

impl FileEntry {
    pub fn from_path(path: PathBuf) -> std::io::Result<Self> {
        let meta = std::fs::symlink_metadata(&path)?;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let is_hidden = name.starts_with('.');
        let is_dir = meta.is_dir();
        let is_symlink = meta.file_type().is_symlink();
        // For symlink, try to check target is_dir
        let is_dir = if is_symlink {
            std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(is_dir)
        } else {
            is_dir
        };
        let size = if is_dir { 0 } else { meta.len() };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let (icon_name, mime_approx) = guess_icon_and_mime(&path, is_dir, is_symlink);

        Ok(Self {
            name,
            path,
            is_dir,
            is_symlink,
            is_hidden,
            size,
            modified,
            icon_name,
            mime_approx,
        })
    }
}

fn guess_icon_and_mime(path: &Path, is_dir: bool, is_symlink: bool) -> (String, String) {
    if is_symlink {
        // Check broken symlink
        if !path.exists() {
            return ("text-x-generic".to_string(), "inode/symlink (broken)".to_string());
        }
    }
    if is_dir {
        return ("folder".to_string(), "inode/directory".to_string());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string();

    let icon = match ext.as_str() {
        "rs" => "text-x-rust",
        "py" => "text-x-python",
        "js" | "ts" | "jsx" | "tsx" => "text-x-javascript",
        "html" | "htm" => "text-html",
        "css" => "text-css",
        "json" => "text-json",
        "toml" | "yaml" | "yml" => "text-x-generic",
        "md" | "markdown" => "text-x-generic",
        "txt" | "log" => "text-plain",
        "pdf" => "application-pdf",
        "zip" | "tar" | "gz" | "xz" | "7z" | "rar" | "bz2" => "package-x-generic",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" => "image-x-generic",
        "mp3" | "flac" | "wav" | "ogg" | "m4a" => "audio-x-generic",
        "mp4" | "mkv" | "avi" | "webm" | "mov" => "video-x-generic",
        "sh" | "bash" => "application-x-shellscript",
        "c" | "cpp" | "h" | "hpp" => "text-x-csrc",
        "nix" => "text-x-script",
        _ => {
            if mime.starts_with("image/") {
                "image-x-generic"
            } else if mime.starts_with("video/") {
                "video-x-generic"
            } else if mime.starts_with("audio/") {
                "audio-x-generic"
            } else if mime.starts_with("text/") {
                "text-plain"
            } else {
                "text-x-generic"
            }
        }
    };
    (icon.to_string(), mime)
}

pub fn load_directory(
    path: &Path,
    show_hidden: bool,
    search: &str,
) -> std::io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    let read = std::fs::read_dir(path)?;
    let search_lower = search.to_lowercase();
    let has_search = !search_lower.is_empty();
    for ent in read {
        let ent = ent?;
        let p = ent.path();
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let is_hidden = name.starts_with('.');
        if !show_hidden && is_hidden {
            continue;
        }
        if has_search && !name.to_lowercase().contains(&search_lower) {
            continue;
        }
        match FileEntry::from_path(p) {
            Ok(e) => entries.push(e),
            Err(_) => continue,
        }
    }
    // Sort: dirs first, then by name case-insensitive
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}

pub fn format_time(t: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Local> = t.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}
