//! Durable tool media, and bounded, range-aware delivery to the WebView.
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{
    http::{Request, Response},
    AppHandle, Manager,
};
use tauri_plugin_opener::OpenerExt;

const CHUNK: u64 = 1024 * 1024;
const BLOB_LIMIT: usize = 10 * 1024 * 1024;
const MAX_MEDIA: usize = 12;
static PREVIEW_WORK: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Clone)]
struct Entry {
    token: String,
    path: PathBuf,
    mime: String,
    image: bool,
}
#[derive(Default)]
pub struct Media {
    files: Mutex<VecDeque<Entry>>,
}

pub fn format(path: &str) -> Option<(&'static str, &'static str)> {
    let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "gif" => ("image", "image/gif"),
        "webp" => ("image", "image/webp"),
        "svg" => ("image", "image/svg+xml"),
        "bmp" => ("image", "image/bmp"),
        "mp4" | "m4v" => ("video", "video/mp4"),
        "webm" => ("video", "video/webm"),
        "mov" => ("video", "video/quicktime"),
        "mkv" => ("video", "video/x-matroska"),
        "mp3" => ("audio", "audio/mpeg"),
        "wav" => ("audio", "audio/wav"),
        "ogg" | "oga" => ("audio", "audio/ogg"),
        "flac" => ("audio", "audio/flac"),
        "m4a" => ("audio", "audio/mp4"),
        _ => return None,
    })
}

fn local_path(value: &str, project: Option<&Path>) -> Result<PathBuf, String> {
    let path = if value.to_ascii_lowercase().starts_with("file:") {
        tauri::Url::parse(value)
            .map_err(|_| "媒体路径无效")?
            .to_file_path()
            .map_err(|_| "媒体路径无效")?
    } else {
        PathBuf::from(value)
    };
    let text = path.to_string_lossy();
    // Network shares can block indefinitely and are not local media.
    if text.starts_with("//")
        || text.to_ascii_lowercase().starts_with(r"\\?\unc\")
        || text.starts_with(r"\\") && !text.starts_with(r"\\?\")
    {
        return Err("请使用本地媒体文件".into());
    }
    let path = if path.is_absolute() {
        path
    } else {
        project.ok_or("相对媒体路径需要所属项目")?.join(path)
    };
    let path = path
        .canonicalize()
        .map_err(|_| "媒体文件不存在或无法读取")?;
    let info = fs::metadata(&path).map_err(|_| "媒体文件不存在或无法读取")?;
    if !info.is_file() || info.len() == 0 {
        return Err("媒体文件为空或不是普通文件".into());
    }
    format(&path.to_string_lossy()).ok_or("不支持预览此文件类型")?;
    Ok(path)
}

fn resolve_file(
    app: &AppHandle,
    path: &str,
    project_id: Option<String>,
) -> Result<PathBuf, String> {
    let state = app.state::<crate::AppState>();
    let project = project_id
        .filter(|id| !id.is_empty())
        .map(|id| state.store.project(&id).map(|p| PathBuf::from(p.path)))
        .transpose()
        .map_err(|_| "媒体所属项目不存在")?;
    local_path(path, project.as_deref())
}

#[tauri::command]
pub async fn open_media(
    path: String,
    project_id: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Cached screenshots and user-provided absolute media may be outside the
        // project. Only known media files can reach the OS document opener.
        let path = resolve_file(&app, &path, project_id)?;
        app.opener()
            .open_path(path.to_string_lossy().into_owned(), None::<&str>)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

impl Media {
    fn register(&self, path: PathBuf, mime: &str, image: bool) -> Result<String, String> {
        let token = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
        let mut files = self.files.lock().map_err(|_| "媒体加载失败")?;
        if let Some(index) = files.iter().position(|file| file.token == token) {
            files.remove(index);
        }
        files.push_back(Entry {
            token: token.clone(),
            path,
            mime: mime.into(),
            image,
        });
        while files.len() > 256 {
            files.pop_front();
        }
        Ok(token)
    }
}

#[tauri::command]
pub async fn prepare_media(
    path: String,
    project_id: Option<String>,
    app: AppHandle,
) -> Result<Value, String> {
    let _permit = PREVIEW_WORK.acquire().await.map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::AppState>();
        let path = resolve_file(&app, &path, project_id)?;
        let (kind, mime) = format(&path.to_string_lossy()).ok_or("不支持预览此文件类型")?;
        if kind == "image" && fs::metadata(&path).map_err(|e| e.to_string())?.len() > BLOB_LIMIT as u64 { return Err("图片超过 10 MB，请在系统中打开".into()); }
        let registry = app.state::<Media>();
        let token = registry.register(path.clone(), mime, kind == "image")?;
        let mut preview = token.clone();
        // Raster previews decode off the UI thread and cache a 960px image.
        if matches!(mime, "image/png" | "image/jpeg" | "image/webp") {
            let root = state.data_dir.join("media");
            fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let bytes = crate::image_attachments::read_image(&path, BLOB_LIMIT as u64)?;
            let key = format!("{:x}", Sha256::digest(&bytes));
            let thumb = root.join(format!("{key}.preview.png"));
            if !thumb.exists() { fs::write(&thumb, crate::image_attachments::thumbnail_size(&bytes, 960, 720)?).map_err(|e| e.to_string())?; }
            preview = registry.register(thumb, "image/png", true)?;
        }
        Ok(json!({"token":token,"previewToken":preview,"kind":kind,"mimeType":mime,"path":path.to_string_lossy(),"name":path.file_name().unwrap_or_default().to_string_lossy()}))
    }).await.map_err(|e| e.to_string())?
}

fn byte_range(range: Option<&str>, length: u64) -> Result<(u64, u64, bool), ()> {
    if length == 0 {
        return Err(());
    }
    let Some(range) = range else {
        return Ok((0, length, false));
    };
    let range = range.strip_prefix("bytes=").ok_or(())?;
    if range.contains(',') {
        return Err(());
    }
    let (from, to) = range.split_once('-').ok_or(())?;
    let (start, end) = if from.is_empty() {
        let suffix: u64 = to.parse().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        (length.saturating_sub(suffix), length - 1)
    } else {
        let start: u64 = from.parse().map_err(|_| ())?;
        let end = if to.is_empty() {
            length - 1
        } else {
            to.parse::<u64>().map_err(|_| ())?.min(length - 1)
        };
        (start, end)
    };
    if start >= length || end < start {
        return Err(());
    }
    Ok((start, (end - start + 1).min(CHUNK), true))
}

pub fn response(media: &Media, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let result = || -> Result<Response<Vec<u8>>, String> {
        if !matches!(request.method().as_str(), "GET" | "HEAD") {
            return Err("不支持此媒体请求".into());
        }
        let token = request.uri().path().trim_start_matches('/');
        let entry = media
            .files
            .lock()
            .map_err(|_| "媒体加载失败")?
            .iter()
            .find(|file| file.token == token)
            .cloned()
            .ok_or("媒体尚未授权")?;
        let mut file = fs::File::open(&entry.path).map_err(|_| "媒体文件已移动或删除")?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let mut builder = Response::builder()
            .header("Content-Type", entry.mime)
            .header("Accept-Ranges", "bytes")
            .header("X-Content-Type-Options", "nosniff")
            .header("Access-Control-Allow-Origin", "*")
            .header("Cache-Control", "no-cache");
        if request.method() == "HEAD" {
            return builder
                .header("Content-Length", len)
                .body(vec![])
                .map_err(|e| e.to_string());
        }
        let (start, count, partial) = match byte_range(
            request.headers().get("range").and_then(|h| h.to_str().ok()),
            len,
        ) {
            Ok(range) => range,
            Err(_) => {
                return builder
                    .status(416)
                    .header("Content-Range", format!("bytes */{len}"))
                    .body(vec![])
                    .map_err(|e| e.to_string())
            }
        };
        // A video without Range must not read an entire recording into memory.
        if !partial
            && len
                > if entry.image {
                    BLOB_LIMIT as u64
                } else {
                    CHUNK
                }
        {
            return Err("大媒体需要分段读取".into());
        }
        if partial {
            builder = builder.status(206).header(
                "Content-Range",
                format!("bytes {start}-{}/{len}", start + count - 1),
            );
        }
        file.seek(SeekFrom::Start(start))
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::with_capacity(count as usize);
        file.take(count)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        builder
            .header("Content-Length", bytes.len())
            .body(bytes)
            .map_err(|e| e.to_string())
    };
    result().unwrap_or_else(|_| {
        Response::builder()
            .status(404)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body("无法加载媒体".as_bytes().to_vec())
            .unwrap()
    })
}

fn cache_blob(
    root: &Path,
    encoded: &str,
    mime: &str,
    name: Option<&str>,
    budget: &mut usize,
) -> Result<Value, String> {
    let extension = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" => "ogg",
        "audio/mp4" => "m4a",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        _ => return Err("工具返回的媒体格式暂不支持".into()),
    };
    let limit = BLOB_LIMIT.min(*budget);
    if encoded.len() > ((limit + 2) / 3) * 4 {
        return Err("工具返回的媒体过大".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "工具返回的媒体数据无效")?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err("工具返回的媒体为空或过大".into());
    }
    if mime.starts_with("image/") && crate::image_attachments::image_format(&bytes)?.1 != mime {
        return Err("工具返回的图片类型与内容不一致".into());
    }
    *budget -= bytes.len();
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let path = root.join(format!("{:x}.{extension}", Sha256::digest(&bytes)));
    if !path.exists() {
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
    }
    let kind = format(&path.to_string_lossy()).unwrap().0;
    Ok(
        json!({"type":"media","kind":kind,"path":path.to_string_lossy(),"mimeType":mime,"name":name.map(|n| n.chars().filter(|c| !c.is_control()).take(160).collect::<String>()).unwrap_or_else(|| format!("{}.{extension}",if kind=="image"{"截图"}else if kind=="video"{"视频"}else{"音频"}))}),
    )
}

/// Replace binary blocks before either IPC or persistence; text remains intact.
pub fn normalize_item(root: &Path, item: &mut Value) {
    fn visit(
        root: &Path,
        value: &mut Value,
        refs: &mut Vec<Value>,
        errors: &mut Vec<String>,
        budget: &mut usize,
        depth: usize,
    ) {
        if depth > 12 {
            return;
        }
        let kind = value["type"].as_str().unwrap_or("");
        let binary = matches!(kind, "image" | "image_url" | "audio" | "video")
            || kind == "resource" && value["resource"]["blob"].is_string();
        let source = value["url"]
            .as_str()
            .or_else(|| value["image_url"]["url"].as_str())
            .or_else(|| value["image_url"].as_str())
            .or_else(|| value["source"]["url"].as_str())
            .or_else(|| value["resource"]["uri"].as_str())
            .or_else(|| value["uri"].as_str())
            .or_else(|| value["path"].as_str());
        if binary {
            let block = if kind == "resource" {
                &value["resource"]
            } else {
                &*value
            };
            let data_url = source
                .filter(|s| s.starts_with("data:"))
                .and_then(|s| s.strip_prefix("data:")?.split_once(";base64,"));
            let encoded = block["data"]
                .as_str()
                .or_else(|| block["source"]["data"].as_str())
                .or_else(|| block["blob"].as_str())
                .or_else(|| data_url.map(|(_, data)| data));
            if let Some(encoded) = encoded {
                let mime = data_url
                    .map(|(mime, _)| mime)
                    .or_else(|| block["mimeType"].as_str())
                    .or_else(|| block["source"]["media_type"].as_str())
                    .unwrap_or(if matches!(kind, "image" | "image_url") {
                        "image/png"
                    } else {
                        ""
                    });
                let cached = if refs.len() >= MAX_MEDIA {
                    Err("单次工具结果最多展示 12 个媒体".into())
                } else {
                    cache_blob(root, encoded, mime, block["name"].as_str(), budget)
                };
                match cached {
                    Ok(media) => {
                        refs.push(media.clone());
                        *value = media;
                    }
                    Err(error) => {
                        errors.push(error.clone());
                        *value = json!({"type":"mediaError","message":error});
                    }
                }
                return;
            }
        }
        if let Some(source) = source {
            let source = source.to_owned();
            let path = tauri::Url::parse(&source)
                .ok()
                .map(|u| u.path().to_owned())
                .unwrap_or_else(|| source.clone());
            let hint = match kind {
                "image" | "image_url" => Some(("image", "image/png")),
                "video" => Some(("video", "video/mp4")),
                "audio" => Some(("audio", "audio/mpeg")),
                _ => None,
            };
            if let Some((kind, mime)) = format(&path).or(hint) {
                if !source.starts_with("data:") && source.len() <= 8192 && refs.len() < MAX_MEDIA {
                    refs.push(json!({"type":"media","kind":kind,"path":source,"mimeType":mime,"name":value["name"].as_str().unwrap_or("媒体")}));
                }
            }
        }
        if binary {
            // Unsupported/malformed binary sources must not leak into IPC or storage.
            *value = json!({"type":"attachment","note":"媒体已处理，或来源无法预览"});
            return;
        }
        match value {
            Value::Array(values) => {
                for v in values.iter_mut().take(100) {
                    visit(root, v, refs, errors, budget, depth + 1);
                }
            }
            Value::Object(values) => {
                for (_, v) in values.iter_mut().take(100) {
                    visit(root, v, refs, errors, budget, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut refs = vec![];
    let mut errors = vec![];
    let mut budget = 20 * 1024 * 1024;
    for key in ["result", "contentItems", "content", "structuredContent"] {
        if let Some(value) = item.get_mut(key) {
            visit(root, value, &mut refs, &mut errors, &mut budget, 0);
        }
    }
    if item["type"] == "imageView" {
        if let Some(path) = item["path"].as_str() {
            refs.push(json!({"type":"media","kind":"image","path":path,"name":"图片"}));
        }
    }
    if !refs.is_empty() {
        let mut paths = std::collections::HashSet::new();
        refs.retain(|r| paths.insert(r["path"].as_str().unwrap_or("").to_owned()));
        item["media"] = json!(refs);
    }
    if !errors.is_empty() {
        item["mediaErrors"] = json!(errors);
    }
}

pub fn normalize_event(app: &AppHandle, event: &mut Value) {
    if matches!(
        event["method"].as_str(),
        Some("item/started" | "item/updated" | "item/completed")
    ) {
        let root = app.state::<crate::AppState>().data_dir.join("media");
        normalize_item(&root, &mut event["params"]["item"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeking_is_bounded_and_invalid_ranges_are_rejected() {
        assert_eq!(
            byte_range(Some("bytes=100-"), 8 * CHUNK),
            Ok((100, CHUNK, true))
        );
        assert_eq!(byte_range(Some("bytes=-20"), 100), Ok((80, 20, true)));
        assert_eq!(byte_range(Some("bytes=10-19"), 100), Ok((10, 10, true)));
        for range in [
            "bytes=20-10",
            "bytes=100-",
            "bytes=-0",
            "bytes=0-1,4-5",
            "bad",
        ] {
            assert!(byte_range(Some(range), 100).is_err());
        }
    }
    #[test]
    fn tool_images_survive_text_bounding_and_reopening() {
        let root = std::env::temp_dir().join(format!("supercode-media-{}", uuid::Uuid::new_v4()));
        let mut bytes = std::io::Cursor::new(vec![]);
        image::DynamicImage::new_rgb8(12, 8)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(bytes.get_ref());
        for block in [
            json!({"type":"image","data":data,"mimeType":"image/png"}),
            json!({"type":"image","source":{"type":"base64","data":data,"media_type":"image/png"}}),
            json!({"type":"image_url","image_url":{"url":format!("data:image/png;base64,{data}")}}),
            json!({"type":"resource","resource":{"uri":"file:///shot.png","blob":data,"mimeType":"image/png"}}),
        ] {
            let mut item = json!({"type":"claudeToolCall","id":"shot","contentItems":[{"type":"text","text":"已截图"},block]});
            normalize_item(&root, &mut item);
            assert_eq!(item["media"][0]["kind"], "image");
            assert!(Path::new(item["media"][0]["path"].as_str().unwrap()).is_file());
            assert!(!item.to_string().contains(&data));
            assert_eq!(item["contentItems"][0]["text"], "已截图");
            let (_, _, _, saved) = crate::protocol::item_message(&item).unwrap();
            assert_eq!(saved["media"][0]["path"], item["media"][0]["path"]);
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn binary_errors_are_explained_without_leaking_base64() {
        let mut item = json!({"type":"mcpToolCall","result":{"content":[{"type":"image","data":"bad-data","mimeType":"image/png"}]}});
        normalize_item(Path::new("unused"), &mut item);
        assert!(item["mediaErrors"].is_array());
        assert!(!item.to_string().contains("bad-data"));
    }
    #[test]
    fn extensionless_remote_images_and_large_tool_metadata_keep_references() {
        let mut item = json!({"type":"mcpToolCall","id":"shot","arguments":(0..40).map(|i|(i.to_string(),json!("x".repeat(32768)))).collect::<serde_json::Map<_,_>>(),"result":{"content":[{"type":"image","url":"https://example.com/screenshot"}]}});
        normalize_item(Path::new("unused"), &mut item);
        let (_, _, _, saved) = crate::protocol::item_message(&item).unwrap();
        assert_eq!(saved["media"][0]["path"], "https://example.com/screenshot");
        assert!(saved.to_string().len() < 140 * 1024);
    }
    #[test]
    fn only_registered_media_is_served_and_range_requests_seek() {
        let root = std::env::temp_dir().join(format!("supercode-media-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("片段.mp4");
        fs::write(&path, (0..100u8).collect::<Vec<_>>()).unwrap();
        let registry = Media::default();
        let token = registry.register(path.clone(), "video/mp4", false).unwrap();
        let request = Request::builder()
            .uri(format!("http://supercode-media.localhost/{token}"))
            .header("Range", "bytes=10-19")
            .body(vec![])
            .unwrap();
        let result = response(&registry, request);
        assert_eq!(result.status(), 206);
        assert_eq!(result.headers()["Content-Range"], "bytes 10-19/100");
        assert_eq!(result.body(), &(10..20u8).collect::<Vec<_>>());
        assert_eq!(
            response(
                &registry,
                Request::builder()
                    .uri("http://supercode-media.localhost/C:/secret.txt")
                    .body(vec![])
                    .unwrap()
            )
            .status(),
            404
        );
        fs::File::create(&path).unwrap().set_len(8 * CHUNK).unwrap();
        let head = response(
            &registry,
            Request::builder()
                .method("HEAD")
                .uri(format!("http://supercode-media.localhost/{token}"))
                .body(vec![])
                .unwrap(),
        );
        assert!(head.body().is_empty());
        assert_eq!(head.headers()["Content-Length"], (8 * CHUNK).to_string());
        let range = response(
            &registry,
            Request::builder()
                .uri(format!("http://supercode-media.localhost/{token}"))
                .header("Range", "bytes=0-")
                .body(vec![])
                .unwrap(),
        );
        assert_eq!(range.status(), 206);
        assert_eq!(range.body().len(), CHUNK as usize);
        assert_eq!(
            response(
                &registry,
                Request::builder()
                    .uri(format!("http://supercode-media.localhost/{token}"))
                    .body(vec![])
                    .unwrap()
            )
            .status(),
            404
        );
        assert!(local_path("片段.mp4", Some(&root)).is_ok());
        assert!(local_path("片段.mp4", None).is_err());
        assert!(local_path("//server/file.mp4", None).is_err());
        let script = root.join("script.ps1");
        fs::write(&script, "Write-Host test").unwrap();
        assert!(local_path(script.to_str().unwrap(), None).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
