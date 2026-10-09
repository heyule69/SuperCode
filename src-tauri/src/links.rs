use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

fn external_url(value: &str) -> Result<String, String> {
    let url = tauri::Url::parse(value).map_err(|_| "链接无效")?;
    if !matches!(url.scheme(), "http" | "https" | "mailto")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("不支持此链接".into());
    }
    Ok(url.into())
}

#[tauri::command]
pub fn open_external_link(app: AppHandle, url: String) -> Result<(), String> {
    app.opener()
        .open_url(external_url(&url)?, None::<String>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_links_cannot_open_programs_files_or_embedded_credentials() {
        for value in [
            "file:///C:/Windows/a.exe",
            "javascript:alert(1)",
            "data:text/html,x",
            "https://user:key@example.com",
            "not a url",
        ] {
            assert!(external_url(value).is_err());
        }
        assert_eq!(
            external_url("https://example.com/docs").unwrap(),
            "https://example.com/docs"
        );
        assert!(external_url("mailto:help@example.com").is_ok());
    }
}
