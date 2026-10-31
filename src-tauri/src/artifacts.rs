//! Final file references and metadata-only snapshots of the designated outputs directory.
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    time::SystemTime,
};
use tauri::{AppHandle, Manager};

const MAX_REFERENCES: usize = 64;
const MAX_OUTPUT_FILES: usize = 512;

#[derive(Clone, Serialize)]
pub struct Artifact {
    pub path: String,
    pub name: String,
    pub size: u64,
}

fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(path) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{path}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
}

fn source_path(value: &str, root: Option<&Path>) -> Result<PathBuf, String> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err("产物文件路径无效".into());
    }
    let path = if value.to_ascii_lowercase().starts_with("file:") {
        tauri::Url::parse(value)
            .map_err(|_| "产物文件路径无效")?
            .to_file_path()
            .map_err(|_| "产物文件路径无效")?
    } else {
        PathBuf::from(value)
    };
    Ok(if path.is_absolute() {
        path
    } else {
        root.ok_or("相对产物路径需要工作目录")?.join(path)
    })
}

fn resolve(paths: &[String], root: Option<&Path>) -> Result<Vec<Artifact>, String> {
    if paths.len() > MAX_REFERENCES {
        return Err("单轮最多展示 64 个产物文件".into());
    }
    let mut files = BTreeMap::new();
    for value in paths {
        let path = source_path(value, root)?;
        let path = match path.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("无法访问产物 {}：{error}", path.display())),
        };
        let info = std::fs::metadata(&path).map_err(|error| error.to_string())?;
        if !info.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .ok_or("产物文件名称无效")?
            .to_string_lossy()
            .into_owned();
        files.insert(
            path.clone(),
            Artifact {
                path: display_path(&path),
                name,
                size: info.len(),
            },
        );
    }
    Ok(files.into_values().collect())
}

fn workspace(
    app: &AppHandle,
    project_id: &str,
    session_id: Option<&str>,
) -> Result<PathBuf, String> {
    Ok(PathBuf::from(
        app.state::<crate::AppState>()
            .store
            .workspace(project_id, session_id)?
            .path,
    ))
}

#[tauri::command]
pub async fn resolve_artifact_references(
    paths: Vec<String>,
    project_id: String,
    session_id: Option<String>,
    app: AppHandle,
) -> Result<Vec<Artifact>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let relative = paths.iter().any(|path| {
            !Path::new(path).is_absolute() && !path.to_ascii_lowercase().starts_with("file:")
        });
        let root = if relative {
            Some(workspace(&app, &project_id, session_id.as_deref())?)
        } else {
            None
        };
        resolve(&paths, root.as_deref())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), String> {
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    if !source.is_file() {
        return Err("产物不是普通文件".into());
    }
    if destination.canonicalize().is_ok_and(|path| path == source) {
        return Err("另存位置不能与原文件相同".into());
    }
    std::fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| format!("另存产物失败：{error}"))
}

#[tauri::command]
pub async fn save_artifact_copy(
    path: String,
    destination: String,
    project_id: String,
    session_id: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root =
            if Path::new(&path).is_absolute() || path.to_ascii_lowercase().starts_with("file:") {
                None
            } else {
                Some(workspace(&app, &project_id, session_id.as_deref())?)
            };
        let source = source_path(&path, root.as_deref())?;
        let destination = PathBuf::from(destination);
        if !destination.is_absolute() {
            return Err("另存位置必须是绝对路径".into());
        }
        copy_file(&source, &destination)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: SystemTime,
}
type Index = BTreeMap<PathBuf, Stamp>;
struct Baseline {
    root: PathBuf,
    files: Result<Index, String>,
}
#[derive(Default)]
pub struct Tracker {
    baselines: HashMap<(String, String), Baseline>,
}

fn scan(root: &Path) -> Result<Index, String> {
    fn visit(root: &Path, depth: usize, files: &mut Index) -> Result<(), String> {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("无法检查交付目录 {}：{error}", root.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if depth >= 8 {
                    return Err("交付目录层级超过 8 层，未自动收集产物".into());
                }
                visit(&entry.path(), depth + 1, files)?;
            } else if kind.is_file() {
                if files.len() >= MAX_OUTPUT_FILES {
                    return Err("交付目录超过 512 个文件，未自动收集产物".into());
                }
                let metadata = entry.metadata().map_err(|error| error.to_string())?;
                files.insert(
                    entry.path(),
                    Stamp {
                        size: metadata.len(),
                        modified: metadata.modified().map_err(|error| error.to_string())?,
                    },
                );
            }
        }
        Ok(())
    }
    let mut files = Index::new();
    if std::fs::symlink_metadata(root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("交付目录是符号链接，未自动收集产物".into());
    }
    visit(root, 0, &mut files)?;
    Ok(files)
}

impl Tracker {
    pub fn begin(&mut self, session: &str, turn: &str, workspace: &Path) {
        self.baselines
            .entry((session.into(), turn.into()))
            .or_insert_with(|| {
                let root = workspace.join("outputs");
                let files = scan(&root);
                Baseline { root, files }
            });
    }
    pub fn finish(
        &mut self,
        session: &str,
        turn: &str,
        completed: bool,
    ) -> Result<Vec<Artifact>, String> {
        let Some(baseline) = self.baselines.remove(&(session.into(), turn.into())) else {
            return Ok(vec![]);
        };
        if !completed {
            return Ok(vec![]);
        }
        let before = baseline.files?;
        let after = scan(&baseline.root)?;
        let changed = after
            .iter()
            .filter(|(path, stamp)| before.get(*path) != Some(*stamp))
            .map(|(path, _)| display_path(path))
            .collect::<Vec<_>>();
        resolve(&changed, None)
    }
}

pub fn observe(app: &AppHandle, message: &Value) -> Option<Value> {
    let method = message["method"].as_str()?;
    if !matches!(method, "turn/started" | "turn/completed") {
        return None;
    }
    let native = message["params"]["threadId"].as_str()?;
    let turn = message["params"]["turn"]["id"].as_str()?;
    let state = app.state::<crate::AppState>();
    let session = state.store.for_native(native).ok()?;
    let result: Result<Option<Vec<Artifact>>, String> = (|| {
        let mut tracker = state
            .artifact_snapshots
            .lock()
            .map_err(|_| "产物记录锁不可用".to_owned())?;
        if method == "turn/started" {
            tracker.begin(
                &session.id,
                turn,
                Path::new(&state.store.session_workspace(&session)?.path),
            );
            return Ok(None);
        }
        let files = tracker.finish(
            &session.id,
            turn,
            message["params"]["turn"]["status"] == "completed",
        )?;
        Ok(Some(files))
    })();
    let (files, error) = match result {
        Ok(None) => return None,
        Ok(Some(files)) if files.is_empty() => return None,
        Ok(Some(files)) => (files, None),
        Err(error) => {
            crate::runtime_diagnostics::message(
                app,
                Some(&session.id),
                "artifacts",
                "warn",
                "artifact_collection_error",
                &error,
            );
            (vec![], Some(error))
        }
    };
    Some(
        json!({"method":"turn/artifacts/updated","params":{"threadId":native,"appSessionId":session.id,"turnId":turn,"artifacts":files,"error":error}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("supercode-artifacts-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn write(&self, path: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn references_validate_real_files_in_all_formats_without_reading_or_rewriting_contents() {
        let root = Fixture::new();
        for name in [
            "中文目录 with spaces/报告.xlsx",
            "旧表.xls",
            "文档.docx",
            "演示.pptx",
            "archive.7z",
            "unknown.custom",
            "无扩展名",
        ] {
            let bytes = [0xff, 0xfe, 0, 0x81];
            let path = root.write(name, &bytes);
            let files = resolve(&[name.into(), display_path(&path)], Some(&root.0)).unwrap();
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].size, 4);
            assert_eq!(files[0].path, display_path(&path.canonicalize().unwrap()));
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        assert!(resolve(
            &["missing.xlsx".into(), display_path(&root.0)],
            Some(&root.0)
        )
        .unwrap()
        .is_empty());
        let empty = root.write("empty.txt", b"");
        assert_eq!(resolve(&[display_path(&empty)], None).unwrap()[0].size, 0);
    }

    #[test]
    fn normal_chat_and_unchanged_outputs_produce_no_artifact_or_directory() {
        let root = Fixture::new();
        let mut tracker = Tracker::default();
        tracker.begin("s", "chat", &root.0);
        assert!(tracker.finish("s", "chat", true).unwrap().is_empty());
        assert!(!root.0.join("outputs").exists());
        root.write("outputs/old.xlsx", b"existing");
        tracker.begin("s", "unchanged", &root.0);
        assert!(tracker.finish("s", "unchanged", true).unwrap().is_empty());
    }

    #[test]
    fn collects_new_and_modified_deliverables_when_the_model_omits_a_final_link() {
        let root = Fixture::new();
        let mut tracker = Tracker::default();
        root.write("outputs/old.xlsx", b"old");
        root.write("outputs/unchanged.docx", b"same");
        root.write("outputs/deleted.pdf", b"delete");
        tracker.begin("s", "t", &root.0);
        root.write("outputs/old.xlsx", b"updated binary spreadsheet");
        root.write("outputs/中文目录/结果.custom", &[0xff, 0, 1]);
        root.write("work/preview.png", b"intermediate");
        root.write("src/app.ts", b"source change");
        std::fs::remove_file(root.0.join("outputs/deleted.pdf")).unwrap();
        tracker.begin("s", "t", &root.0); // Repeated native start must not reset the baseline.
        let files = tracker.finish("s", "t", true).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|file| file.name == "old.xlsx"));
        assert!(files.iter().any(|file| file.name == "结果.custom"));
        assert!(tracker.finish("s", "t", true).unwrap().is_empty());
        tracker.begin("s", "next", &root.0);
        assert!(tracker.finish("s", "next", true).unwrap().is_empty());
    }

    #[test]
    fn failed_turns_and_other_workspaces_do_not_create_final_file_cards() {
        let main = Fixture::new();
        let side = Fixture::new();
        let mut tracker = Tracker::default();
        tracker.begin("main", "t", &main.0);
        tracker.begin("side", "t", &side.0);
        side.write("outputs/side.xlsx", b"side output");
        assert!(tracker.finish("main", "t", true).unwrap().is_empty());
        assert!(tracker.finish("side", "t", false).unwrap().is_empty());
        assert!(tracker.baselines.is_empty());
    }

    #[test]
    fn save_as_copies_bytes_and_never_overwrites_the_original_with_itself() {
        let root = Fixture::new();
        let bytes = [0xff, 0xfe, 0, 0x81];
        let source = root.write("原始文件.xlsx", &bytes);
        let destination = root.0.join("另存副本.xlsx");
        copy_file(&source, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), bytes);
        assert!(copy_file(&source, &source)
            .unwrap_err()
            .contains("原文件相同"));
        assert_eq!(std::fs::read(source).unwrap(), bytes);
    }

    #[test]
    fn scan_errors_are_reported_instead_of_treating_a_broken_output_directory_as_empty() {
        let root = Fixture::new();
        let mut tracker = Tracker::default();
        root.write("outputs", b"not a directory");
        tracker.begin("s", "t", &root.0);
        assert!(tracker
            .finish("s", "t", true)
            .err()
            .unwrap()
            .contains("无法检查交付目录"));
        assert!(resolve(&vec!["file".into(); 65], Some(&root.0)).is_err());
    }
}
