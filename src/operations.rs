use std::path::{Path, PathBuf};

pub fn trash_path(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

pub fn delete_permanent(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

pub fn rename_path(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|e| e.to_string())
}

pub fn create_dir(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let p = parent.join(name);
    std::fs::create_dir(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

pub fn create_file(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let p = parent.join(name);
    std::fs::File::create(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

pub fn copy_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        for ent in std::fs::read_dir(src).map_err(|e| e.to_string())? {
            let ent = ent.map_err(|e| e.to_string())?;
            let file_name = ent.file_name();
            copy_recursive(&ent.path(), &dst.join(file_name))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(src, dst).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn unique_name(parent: &Path, base: &str) -> String {
    let base_path = Path::new(base);
    let stem = base_path.file_stem().and_then(|s| s.to_str()).unwrap_or(base);
    let ext = base_path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let mut candidate = base.to_string();
    let mut n = 1;
    while parent.join(&candidate).exists() {
        if ext.is_empty() {
            candidate = format!("{stem} ({n})");
        } else {
            candidate = format!("{stem} ({n}).{ext}");
        }
        n += 1;
    }
    candidate
}

pub fn open_with_default(path: &Path) -> Result<(), String> {
    open::that(path).map_err(|e| e.to_string())
}
