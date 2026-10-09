use base64::Engine;
use image::{ImageFormat, ImageReader, Limits};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

pub const IMAGE_LIMIT: u64 = 10 * 1024 * 1024;
pub const TOTAL_IMAGE_LIMIT: u64 = 20 * 1024 * 1024;
// Decode one image at a time, off the UI thread. Conversation thumbnails never
// transfer the original image through IPC.
static IMAGE_WORK: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedImage {
    kind: &'static str,
    path: String,
    name: String,
    size: u64,
}

pub fn image_format(bytes: &[u8]) -> Result<(ImageFormat, &'static str, &'static str), String> {
    match image::guess_format(bytes) {
        Ok(ImageFormat::Png) => Ok((ImageFormat::Png, "image/png", "png")),
        Ok(ImageFormat::Jpeg) => Ok((ImageFormat::Jpeg, "image/jpeg", "jpg")),
        Ok(ImageFormat::Gif) => Ok((ImageFormat::Gif, "image/gif", "gif")),
        Ok(ImageFormat::WebP) => Ok((ImageFormat::WebP, "image/webp", "webp")),
        _ => Err("仅支持 PNG、JPEG、GIF、WebP 图片".into()),
    }
}

pub fn read_image(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|_| "图片不存在或无法读取".to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("请选择图片文件".into());
    }
    if file.metadata().map_err(|e| e.to_string())?.len() > limit {
        return Err(if limit < IMAGE_LIMIT {
            "图片总量不能超过 20 MB"
        } else {
            "图片不能超过 10 MB"
        }
        .into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("图片过大，请选择较小的图片".into());
    }
    Ok(bytes)
}

fn thumbnail(bytes: &[u8]) -> Result<Vec<u8>, String> {
    thumbnail_size(bytes, 256, 192)
}

pub(crate) fn thumbnail_size(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let (format, _, _) = image_format(bytes)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "图片损坏或尺寸过大，请换一张图片".to_string())?;
    let mut output = Cursor::new(Vec::new());
    decoded
        .thumbnail(width, height)
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(output.into_inner())
}

fn image_key(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn thumbnail_path(root: &Path, bytes: &[u8]) -> PathBuf {
    root.join(format!("{}.thumb.png", image_key(bytes)))
}
fn data_url(bytes: &[u8], mime: &str) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn import(
    root: &Path,
    path: Option<String>,
    data: Option<String>,
    name: Option<String>,
    remaining: u64,
) -> Result<ImportedImage, String> {
    let limit = IMAGE_LIMIT.min(remaining);
    let bytes = match (path.as_deref(), data.as_deref()) {
        (Some(path), None) => read_image(Path::new(path), limit)?,
        (None, Some(data)) => {
            if data.len() as u64 > ((limit + 2) / 3) * 4 {
                return Err("图片过大或图片总量超过 20 MB".into());
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| "图片数据无效".to_string())?;
            if bytes.len() as u64 > limit {
                return Err("图片过大或图片总量超过 20 MB".into());
            }
            bytes
        }
        _ => return Err("请选择图片或粘贴图片".into()),
    };
    let (_, _, extension) = image_format(&bytes)?;
    let thumb = thumbnail(&bytes)?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let target = root.join(format!("{}.{}", image_key(&bytes), extension));
    // Names are display metadata only, never part of a filesystem target.
    let name = name
        .or_else(|| {
            path.as_deref()
                .and_then(|p| Path::new(p).file_name())
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| format!("粘贴的图片.{extension}"));
    let name = name
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect::<String>();
    if !target.exists() {
        fs::write(&target, &bytes).map_err(|e| e.to_string())?;
    }
    fs::write(thumbnail_path(root, &bytes), thumb).map_err(|e| e.to_string())?;
    Ok(ImportedImage {
        kind: "image",
        path: target.to_string_lossy().into_owned(),
        name,
        size: bytes.len() as u64,
    })
}

#[tauri::command]
pub async fn import_image_attachment(
    path: Option<String>,
    data: Option<String>,
    name: Option<String>,
    remaining_bytes: Option<u64>,
    app: AppHandle,
) -> Result<ImportedImage, String> {
    let root = app.state::<crate::AppState>().data_dir.join("attachments");
    let _permit = IMAGE_WORK.acquire().await.map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        import(
            &root,
            path,
            data,
            name,
            remaining_bytes
                .unwrap_or(TOTAL_IMAGE_LIMIT)
                .min(TOTAL_IMAGE_LIMIT),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn read_image_attachment(
    path: String,
    full: bool,
    app: AppHandle,
) -> Result<String, String> {
    let root = app.state::<crate::AppState>().data_dir.join("attachments");
    let _permit = IMAGE_WORK.acquire().await.map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = read_image(Path::new(&path), IMAGE_LIMIT)?;
        let (_, mime, _) = image_format(&bytes)?;
        if full {
            return Ok(data_url(&bytes, mime));
        }
        let target = thumbnail_path(&root, &bytes);
        let thumb = match fs::read(&target) {
            Ok(bytes) => bytes,
            Err(_) => {
                let thumb = thumbnail(&bytes)?;
                fs::create_dir_all(&root).map_err(|e| e.to_string())?;
                fs::write(target, &thumb).map_err(|e| e.to_string())?;
                thumb
            }
        };
        Ok(data_url(&thumb, "image/png"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let image = image::DynamicImage::new_rgb8(640, 480);
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }
    #[test]
    fn pasted_and_uploaded_images_are_durable_and_identical() {
        let root = std::env::temp_dir().join(format!("supercode-image-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let original = root.join("原图.png");
        let bytes = png();
        fs::write(&original, &bytes).unwrap();
        let cache = root.join("cache");
        let upload = import(
            &cache,
            Some(original.to_string_lossy().into()),
            None,
            None,
            TOTAL_IMAGE_LIMIT,
        )
        .unwrap();
        let pasted = import(
            &cache,
            None,
            Some(base64::engine::general_purpose::STANDARD.encode(&bytes)),
            Some("../../粘贴.png".into()),
            TOTAL_IMAGE_LIMIT,
        )
        .unwrap();
        assert_eq!(upload.path, pasted.path);
        assert_eq!(upload.name, "原图.png");
        fs::remove_file(original).unwrap();
        assert_eq!(fs::read(&upload.path).unwrap(), bytes);
        let thumb =
            image::load_from_memory(&fs::read(thumbnail_path(&cache, &bytes)).unwrap()).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (256, 192));
        let prepared = crate::client_features::prepare_input(
            "看图",
            &[crate::client_features::Attachment {
                kind: "image".into(),
                path: upload.path.clone(),
                name: upload.name.clone(),
            }],
        )
        .unwrap();
        assert_eq!(prepared.codex[1]["type"], "localImage");
        assert_eq!(
            prepared.claude[1]["source"]["data"],
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        assert_eq!(prepared.metadata["attachments"][0]["name"], "原图.png");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_corrupt_and_over_budget_images_are_rejected_before_writing() {
        let root = std::env::temp_dir().join(format!("supercode-image-{}", uuid::Uuid::new_v4()));
        for bytes in [&b"plain text"[..], &b"\x89PNG\r\n\x1a\ncorrupt"[..]] {
            assert!(import(
                &root,
                None,
                Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
                None,
                TOTAL_IMAGE_LIMIT
            )
            .is_err());
        }
        assert!(import(
            &root,
            None,
            Some("invalid base64".into()),
            None,
            TOTAL_IMAGE_LIMIT
        )
        .is_err());
        assert!(import(
            &root,
            None,
            Some(base64::engine::general_purpose::STANDARD.encode(png())),
            None,
            1
        )
        .is_err());
        assert!(!root.exists());
    }
}
