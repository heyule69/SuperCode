//! File-backed observation images. Only this owned directory is eligible for GC.
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
const MAX_IMAGES: usize = 256;
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
pub const DIRECTORY: &str = "process-captures";

fn link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

pub fn directory(media: &Path) -> Result<PathBuf, String> {
    let root = media.join(DIRECTORY);
    match fs::symlink_metadata(&root) {
        Ok(metadata) if link(&metadata) || !metadata.is_dir() => {
            return Err("过程截图缓存目录无效".into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(&root).map_err(|e| e.to_string())?
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(root)
}

pub fn owned(path: &Path, media: &Path) -> bool {
    path.parent() == Some(media.join(DIRECTORY).as_path()) && key(path).is_some()
}

fn key(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let (hash, suffix) = name.split_once('.')?;
    (hash.len() == 64
        && hash.bytes().all(|v| v.is_ascii_hexdigit())
        && matches!(
            suffix,
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "preview.png"
        ))
    .then(|| hash.to_owned())
}

pub fn observation(item: &Value) -> bool {
    if item["type"] == "imageView" {
        return true;
    }
    let tool = item["tool"].as_str().unwrap_or("");
    if item["type"] == "claudeToolCall" && tool == "Read" {
        return true;
    }
    if tool.starts_with("mcp__supercode_browser__browser_")
        || tool.starts_with("mcp__browser__browser_")
        || tool.starts_with("mcp__supercode_computer__desktop_")
        || tool.starts_with("mcp__computer__desktop_")
    {
        return true;
    }
    let server = item["server"]
        .as_str()
        .or_else(|| item["namespace"].as_str())
        .unwrap_or("");
    matches!(server, "browser" | "supercode_browser") && tool.starts_with("browser_")
        || matches!(server, "computer" | "supercode_computer") && tool.starts_with("desktop_")
}

/// Metadata-only scan: no images are decoded or retained in memory.
pub fn prune(root: &Path, now: SystemTime, keep: Option<&Path>) -> Result<(), String> {
    if !root.is_absolute()
        || root.file_name().and_then(|n| n.to_str()) != Some(DIRECTORY)
        || root
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            != Some("media")
    {
        return Err("过程截图清理目录无效".into());
    }
    let metadata = match fs::symlink_metadata(root) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    if link(&metadata) || !metadata.is_dir() {
        return Err("过程截图清理目录不能是链接".into());
    }
    let mut groups: BTreeMap<String, (SystemTime, u64, Vec<(PathBuf, u64)>)> = BTreeMap::new();
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let Some(hash) = key(&path) else {
            continue;
        };
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if link(&metadata) || !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let group = groups.entry(hash).or_insert((modified, 0, Vec::new()));
        group.0 = group.0.max(modified);
        group.1 += metadata.len();
        group.2.push((path, metadata.len()));
    }
    let mut groups = groups.into_iter().collect::<Vec<_>>();
    groups.sort_by_key(|(_, value)| value.0);
    let mut bytes = groups.iter().map(|(_, value)| value.1).sum::<u64>();
    let mut count = groups.len();
    let keep = keep.and_then(key);
    for (hash, (modified, size, files)) in groups {
        if keep.as_ref() == Some(&hash) {
            continue;
        }
        if now.duration_since(modified).unwrap_or_default() <= MAX_AGE
            && bytes <= MAX_BYTES
            && count <= MAX_IMAGES
        {
            continue;
        }
        for (path, _) in files {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        bytes = bytes.saturating_sub(size);
        count = count.saturating_sub(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir()
            .join(format!("supercode-observation-{}", uuid::Uuid::new_v4()))
            .join("media");
        directory(&root).unwrap()
    }
    fn picture(root: &Path, n: usize, bytes: u64, at: SystemTime) -> PathBuf {
        let path = root.join(format!("{n:064x}.jpg"));
        let file = fs::File::create(&path).unwrap();
        file.set_len(bytes).unwrap();
        file.set_modified(at).unwrap();
        path
    }
    fn finish(root: PathBuf) {
        fs::remove_dir_all(root.parent().unwrap().parent().unwrap()).unwrap();
    }
    #[test]
    fn expiration_and_capacity_remove_only_owned_process_files() {
        let root = fixture();
        let now = SystemTime::now();
        let old = picture(&root, 1, 16, now - MAX_AGE - Duration::from_secs(1));
        fs::write(root.join(format!("{:064x}.preview.png", 1)), b"thumbnail").unwrap();
        fs::File::options()
            .write(true)
            .open(root.join(format!("{:064x}.preview.png", 1)))
            .unwrap()
            .set_modified(now - MAX_AGE - Duration::from_secs(1))
            .unwrap();
        let earlier = picture(&root, 2, 30 * 1024 * 1024, now - Duration::from_secs(2));
        let newest = picture(&root, 3, 30 * 1024 * 1024, now);
        fs::write(root.join("user-photo.jpg"), b"keep").unwrap();
        let final_image = root.parent().unwrap().join(format!("{:064x}.jpg", 1));
        fs::write(&final_image, b"final").unwrap();
        prune(&root, now, Some(&newest)).unwrap();
        assert!(!old.exists());
        assert!(!earlier.exists());
        assert!(newest.exists());
        assert!(final_image.exists());
        assert!(root.join("user-photo.jpg").exists());
        finish(root);
    }
    #[test]
    fn too_many_small_observations_cannot_accumulate_unbounded_metadata() {
        let root = fixture();
        let now = SystemTime::now();
        for n in 0..260 {
            picture(&root, n, 1, now - Duration::from_secs(260 - n as u64));
        }
        prune(&root, now, None).unwrap();
        assert_eq!(fs::read_dir(&root).unwrap().count(), MAX_IMAGES);
        finish(root);
    }
    #[test]
    fn ordinary_tool_outputs_and_user_files_are_not_process_cache() {
        assert!(observation(
            &serde_json::json!({"type":"claudeToolCall","tool":"mcp__supercode_browser__browser_snapshot"})
        ));
        assert!(observation(
            &serde_json::json!({"type":"mcpToolCall","server":"supercode_computer","tool":"desktop_click"})
        ));
        assert!(observation(
            &serde_json::json!({"type":"claudeToolCall","tool":"Read"})
        ));
        assert!(!observation(
            &serde_json::json!({"type":"claudeToolCall","tool":"GenerateImage"})
        ));
        assert!(!observation(
            &serde_json::json!({"tool":"browser_snapshot","server":"custom-images"})
        ));
        assert!(prune(Path::new("media/process-captures"), SystemTime::now(), None).is_err());
    }
}
