//! Ordinary attachments are paths, regardless of format, encoding or file size.
use std::path::Path;

pub(crate) fn prepare(path: &Path) -> Result<String, String> {
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("附件不可用 {}：{e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("附件不是文件：{}", path.display()));
    }
    // No content reads, format allowlist, decoding, extraction or file copies.
    Ok(format!(
        "\n\n用户附加的文件（路径：{}，大小：{} 字节）。这是原文件引用，尚未读取正文。请按任务需要使用合适的工具读取或解析，不要仅根据文件名推断内容；不要擅自改写原文件或转换编码。",
        path.display(), metadata.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_features::{prepare_input, Attachment};
    use std::{fs::File, io::Write};

    #[test]
    fn all_formats_are_referenced_without_decoding_or_rewriting() {
        let root =
            std::env::temp_dir().join(format!("supercode-documents-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        for extension in [
            "PDF",
            "doc",
            "docx",
            "xlsx",
            "xls",
            "pptx",
            "ppt",
            "odt",
            "ods",
            "odp",
            "rtf",
            "epub",
            "zip",
            "7z",
            "rar",
            "mp3",
            "wav",
            "mp4",
            "webm",
            "txt",
            "csv",
            "unknown-format",
            "",
        ] {
            let name = if extension.is_empty() {
                "银行数据安全平台需求".to_owned()
            } else {
                format!("银行数据安全平台需求.{extension}")
            };
            let path = root.join(name);
            let original = [0xff, 0xfe, 0, 0x81];
            std::fs::write(&path, original).unwrap();
            let result = prepare_input(
                "分析这个附件",
                &[Attachment {
                    kind: "file".into(),
                    name: path.file_name().unwrap().to_string_lossy().into(),
                    path: path.to_string_lossy().into(),
                }],
            )
            .unwrap();
            let text = result.claude[1]["text"].as_str().unwrap();
            assert!(text.contains("原文件引用"), "{extension}");
            assert!(text.contains("尚未读取正文"));
            assert!(text.contains("银行数据安全平台需求"));
            assert_eq!(result.codex[1], result.claude[1]);
            assert_eq!(result.metadata["attachments"][0]["kind"], "file");
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn large_files_and_small_text_both_remain_small_references() {
        let root =
            std::env::temp_dir().join(format!("supercode-documents-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let pdf = root.join("large.pdf");
        let mut file = File::create(&pdf).unwrap();
        file.write_all(b"%PDF-1.7\n\xff\xfe").unwrap();
        file.set_len(8 * 1024 * 1024).unwrap();
        drop(file);
        let reference = prepare(&pdf).unwrap();
        assert!(reference.contains("8388608 字节"));
        assert!(reference.len() < 2048);
        let csv = root.join("table.csv");
        std::fs::write(&csv, "名称,金额\n测试,123\n").unwrap();
        let reference = prepare(&csv).unwrap();
        assert!(reference.contains("原文件引用"));
        assert!(!reference.contains("测试,123"));
        assert!(prepare(&root).unwrap_err().contains("附件"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_utf8_and_bom_text_pass_through_without_changing_encoding() {
        let path =
            std::env::temp_dir().join(format!("supercode-encoding-{}.txt", uuid::Uuid::new_v4()));
        let bytes = [0xd6, 0xd0, 0xce, 0xc4];
        std::fs::write(&path, bytes).unwrap();
        let reference = prepare(&path).unwrap();
        assert!(reference.contains(path.file_name().unwrap().to_str().unwrap()));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let bom = [0xef, 0xbb, 0xbf, b'A'];
        std::fs::write(&path, bom).unwrap();
        assert!(prepare(&path).unwrap().contains("原文件引用"));
        assert_eq!(std::fs::read(&path).unwrap(), bom);
        std::fs::remove_file(path).unwrap();
    }
}
