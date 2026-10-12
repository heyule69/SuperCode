//! Bounded per-operation snapshots and independent per-turn file baselines.
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
struct Snapshot {
    path: PathBuf,
    before: Option<Arc<str>>,
    operation_id: String,
}
#[derive(Default)]
pub struct Tracker {
    baselines: HashMap<(String, PathBuf), Snapshot>,
    operations: HashMap<(String, String), Vec<Snapshot>>,
}
pub struct TurnChange {
    pub path: String,
    pub diff: String,
}
const FILE_LIMIT: u64 = 256 * 1024;
const CACHE_LIMIT: usize = 2 * 1024 * 1024;

pub fn canonical_target(path: &Path) -> Option<PathBuf> {
    if let Ok(path) = path.canonicalize() {
        return Some(path);
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let mut ancestor = path;
    let mut missing = vec![];
    while !ancestor.exists() {
        missing.push(ancestor.file_name()?.to_owned());
        ancestor = ancestor.parent()?;
    }
    let mut resolved = ancestor.canonicalize().ok()?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Some(resolved)
}
fn content(path: &Path) -> Result<Option<Arc<str>>, String> {
    match std::fs::metadata(path) {
        Ok(_) => {
            crate::client_features::read_utf8(path, FILE_LIMIT).map(|text| Some(Arc::from(text)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("无法读取变更文件".into()),
    }
}
fn targets(item: &Value) -> Vec<String> {
    let args = &item["arguments"];
    let mut paths = ["file_path", "filePath", "path", "notebook_path"]
        .iter()
        .filter_map(|key| args[*key].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    for key in ["patch", "patchText", "input"] {
        if let Some(patch) = args[key].as_str() {
            paths.extend(patch.lines().filter_map(|line| {
                [
                    "*** Add File: ",
                    "*** Update File: ",
                    "*** Delete File: ",
                    "*** Move to: ",
                ]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))
                .map(str::to_owned)
            }));
        }
    }
    paths.retain(|path| {
        !path.is_empty() && path.len() <= 4096 && !path.chars().any(char::is_control)
    });
    paths.sort();
    paths.dedup();
    paths.truncate(32);
    paths
}
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
fn diff(path: &str, before: &str, after: &str) -> (String, usize, usize) {
    let changes = similar::TextDiff::from_lines(before, after);
    let added = changes
        .iter_all_changes()
        .filter(|c| c.tag() == similar::ChangeTag::Insert)
        .count();
    let removed = changes
        .iter_all_changes()
        .filter(|c| c.tag() == similar::ChangeTag::Delete)
        .count();
    let patch = if before == after {
        String::new()
    } else {
        format!(
            "diff --git a/{path} b/{path}\n{}",
            changes
                .unified_diff()
                .context_radius(3)
                .header(&format!("a/{path}"), &format!("b/{path}"))
                .to_string()
        )
    };
    (patch, added, removed)
}
pub fn patch_change(path: &str, patch: &str) -> Option<Value> {
    let mut hunk = false;
    let mut valid = false;
    let mut added = 0;
    let mut removed = 0;
    for line in patch.lines() {
        if line.starts_with("@@ ") {
            hunk = true;
            valid = true;
        } else if line.starts_with("diff --git ") {
            hunk = false;
        } else if hunk && line.starts_with('+') {
            added += 1;
        } else if hunk && line.starts_with('-') {
            removed += 1;
        }
    }
    valid.then(||json!({"path":path,"kind":{"type":"update"},"added":added,"removed":removed,"diff":crate::protocol::bounded(patch,64*1024),"diffTruncated":patch.len()>64*1024}))
}
pub fn summarize(item: &mut Value) {
    if let Some(changes) = item["changes"].as_array() {
        item["changeSummary"]=json!(changes.iter().take(32).map(|c|json!({"path":c["path"],"kind":c["kind"],"added":c["added"],"removed":c["removed"],"diffTruncated":c["diffTruncated"]})).collect::<Vec<_>>());
    }
}
pub fn normalize_native(item: &mut Value) {
    if !item["changes"].is_array() {
        let mut rows = vec![];
        for block in item["contentItems"]
            .as_array()
            .into_iter()
            .flatten()
            .take(32)
        {
            if block["type"] != "diff" {
                continue;
            }
            let Some(path) = block["path"].as_str() else {
                continue;
            };
            let Some(after) = block["newText"]
                .as_str()
                .filter(|text| text.len() <= FILE_LIMIT as usize)
            else {
                continue;
            };
            let before = block["oldText"].as_str().unwrap_or("");
            if before.len() > FILE_LIMIT as usize {
                continue;
            }
            let (patch, added, removed) = diff(path, before, after);
            rows.push(json!({"path":path,"kind":{"type":if block["oldText"].is_null(){"add"}else{"update"}},"added":added,"removed":removed,"diff":crate::protocol::bounded(&patch,64*1024),"diffTruncated":patch.len()>64*1024}));
        }
        if !rows.is_empty() {
            item["changes"] = json!(rows);
        }
    }
    if let Some(changes) = item["changes"].as_array_mut() {
        for change in changes.iter_mut().take(32) {
            if change["added"].is_u64() && change["removed"].is_u64() {
                continue;
            }
            if let (Some(path), Some(patch)) = (change["path"].as_str(), change["diff"].as_str()) {
                if let Some(counted) = patch_change(path, patch) {
                    change["added"] = counted["added"].clone();
                    change["removed"] = counted["removed"].clone();
                }
            }
        }
    }
    summarize(item);
}
impl Tracker {
    pub fn finish_session(&mut self, session: &str) {
        self.baselines.retain(|(owner, _), _| owner != session);
        self.operations.retain(|(owner, _), _| owner != session);
    }
    fn bytes(&self) -> usize {
        self.baselines
            .values()
            .chain(self.operations.values().flatten())
            .map(|s| s.before.as_ref().map_or(0, |s| s.len()))
            .sum()
    }
    pub fn observe(&mut self, session: &str, root: &Path, item: &mut Value) -> Vec<TurnChange> {
        let Some(id) = item["id"].as_str().map(str::to_owned) else {
            return vec![];
        };
        let key = (session.to_owned(), id);
        let Ok(root) = root.canonicalize() else {
            return vec![];
        };
        let terminal = matches!(
            item["status"].as_str(),
            Some("completed" | "failed" | "declined" | "interrupted")
        );
        if !terminal {
            if self.operations.contains_key(&key) {
                return vec![];
            }
            let mut captured = vec![];
            for path in targets(item) {
                let path = if Path::new(&path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    root.join(path)
                };
                let Some(path) = canonical_target(&path).filter(|path| path.starts_with(&root))
                else {
                    continue;
                };
                let before = match content(&path) {
                    Ok(before) => before,
                    Err(error) => {
                        item["changeNote"] = json!(error);
                        continue;
                    }
                };
                let extra = before.as_ref().map_or(0, |s| s.len())
                    * if self
                        .baselines
                        .contains_key(&(session.to_owned(), path.clone()))
                    {
                        1
                    } else {
                        2
                    };
                let pending_bytes: usize = captured
                    .iter()
                    .map(|s: &Snapshot| s.before.as_ref().map_or(0, |s| s.len()))
                    .sum();
                if self.operations.len() >= 64
                    || self.baselines.len() >= 128
                    || self.bytes() + pending_bytes + extra > CACHE_LIMIT
                {
                    item["changeNote"] = json!("变更快照达到缓存上限，保留工具返回的详情");
                    break;
                }
                let snapshot = Snapshot {
                    path: path.clone(),
                    before,
                    operation_id: key.1.clone(),
                };
                self.baselines
                    .entry((session.to_owned(), path))
                    .or_insert_with(|| snapshot.clone());
                captured.push(snapshot);
            }
            if !captured.is_empty() {
                self.operations.insert(key, captured);
            }
            return vec![];
        }
        let native = native_snapshot(item, &root, &key.1);
        let captured = if let Some((snapshot, after)) = native {
            let baseline_key = (session.to_owned(), snapshot.path.clone());
            let current = self.baselines.get(&baseline_key);
            if current.is_none_or(|baseline| baseline.operation_id == key.1) {
                let old_bytes = current
                    .and_then(|s| s.before.as_ref())
                    .map_or(0, |s| s.len());
                let new_bytes = snapshot.before.as_ref().map_or(0, |s| s.len());
                if self.baselines.len() < 128
                    && self.bytes().saturating_sub(old_bytes) + new_bytes <= CACHE_LIMIT
                {
                    self.baselines.insert(baseline_key, snapshot.clone());
                } else {
                    self.baselines.remove(&baseline_key);
                    item["changeNote"] = json!("变更快照达到缓存上限，保留本次操作差异");
                }
            }
            self.operations.remove(&key);
            vec![(snapshot, Ok(after))]
        } else if let Some(captured) = self.operations.remove(&key) {
            captured
                .into_iter()
                .map(|snapshot| {
                    let after = content(&snapshot.path);
                    (snapshot, after)
                })
                .collect()
        } else {
            return vec![];
        };
        let mut rows = vec![];
        let mut turn_changes = vec![];
        for (snapshot, after) in captured {
            let after = match after {
                Ok(after) => after,
                Err(error) => {
                    item["changeNote"] = json!(error);
                    continue;
                }
            };
            let path = relative(&root, &snapshot.path);
            let (patch, added, removed) = diff(
                &path,
                snapshot.before.as_deref().unwrap_or(""),
                after.as_deref().unwrap_or(""),
            );
            let kind = if snapshot.before.is_none() && after.is_some() {
                "add"
            } else if after.is_none() && snapshot.before.is_some() {
                "delete"
            } else {
                "update"
            };
            rows.push(json!({"path":path,"kind":{"type":kind},"added":added,"removed":removed,"diff":crate::protocol::bounded(&patch,64*1024),"diffTruncated":patch.len()>64*1024}));
            if let Some(baseline) = self.baselines.get(&(session.to_owned(), snapshot.path)) {
                let (patch, _, _) = diff(
                    &path,
                    baseline.before.as_deref().unwrap_or(""),
                    after.as_deref().unwrap_or(""),
                );
                turn_changes.push(TurnChange { path, diff: patch });
            }
        }
        // Native per-call patches (e.g. Pi Edit) are authoritative even if the
        // filesystem notification was read just after the write completed.
        if !rows.is_empty() && !item["changes"].is_array() {
            item["changes"] = json!(rows);
        }
        summarize(item);
        turn_changes
    }
}
fn native_snapshot(item: &Value, root: &Path, id: &str) -> Option<(Snapshot, Option<Arc<str>>)> {
    let data = item.get("fileSnapshot")?;
    let path = data["path"].as_str()?;
    let target = canonical_target(&root.join(path)).filter(|path| path.starts_with(root))?;
    // Never use an unrelated returned snapshot to replace this operation's baseline.
    if !targets(item)
        .iter()
        .any(|path| canonical_target(&root.join(path)).as_ref() == Some(&target))
    {
        return None;
    }
    let text = |value: &Value| -> Option<Option<Arc<str>>> {
        if value.is_null() {
            Some(None)
        } else {
            value
                .as_str()
                .filter(|text| text.len() <= FILE_LIMIT as usize)
                .map(|text| Some(Arc::from(text)))
        }
    };
    Some((
        Snapshot {
            path: target,
            before: text(data.get("before")?)?,
            operation_id: id.into(),
        },
        text(data.get("after")?)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_write_snapshots_correct_late_notifications_and_do_not_include_later_edits() {
        let root =
            std::env::temp_dir().join(format!("supercode-late-write-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("a.ts");
        // The native write has already finished when its start reaches the UI.
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let mut tracker = Tracker::default();
        let mut item = json!({"id":"write","tool":"Write","arguments":{"file_path":"a.ts"},"status":"inProgress"});
        tracker.observe("s", &root, &mut item);
        std::fs::write(&path, "a later edit\n").unwrap();
        item["status"] = json!("completed");
        item["fileSnapshot"] = json!({"path":"a.ts","before":null,"after":"one\ntwo\n"});
        let totals = tracker.observe("s", &root, &mut item);
        assert_eq!(item["changeSummary"][0]["kind"]["type"], "add");
        assert_eq!(item["changeSummary"][0]["added"], 2);
        assert_eq!(item["changeSummary"][0]["removed"], 0);
        assert!(totals[0].diff.contains("+two"));
        assert!(!totals[0].diff.contains("later"));
        assert!(native_snapshot(&json!({"arguments":{"file_path":"a.ts"},"fileSnapshot":{"path":"unrelated.ts","before":null,"after":"x"}}),&root,"wrong").is_none());
        assert!(native_snapshot(
            &json!({"arguments":{"file_path":"a.ts"},"fileSnapshot":{"path":"a.ts","after":"x"}}),
            &root,
            "missing"
        )
        .is_none());
        tracker.finish_session("s");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn repeated_edits_keep_per_call_counts_and_separate_turn_totals() {
        let root =
            std::env::temp_dir().join(format!("supercode-edit-details-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("中文.ts");
        let mut tracker = Tracker::default();
        let mut write = json!({"id":"write","tool":"Write","arguments":{"file_path":"中文.ts"},"status":"inProgress"});
        tracker.observe("chat", &root, &mut write);
        std::fs::write(&path, "one\ntwo\n").unwrap();
        write["status"] = "completed".into();
        let turn = tracker.observe("chat", &root, &mut write);
        assert_eq!(write["changes"][0]["kind"]["type"], "add");
        assert_eq!(write["changeSummary"][0]["added"], 2);
        assert!(turn[0].diff.contains("+two"));
        let mut edit = json!({"id":"edit","tool":"Edit","arguments":{"file_path":"中文.ts"},"status":"inProgress"});
        tracker.observe("chat", &root, &mut edit);
        std::fs::write(&path, "one\nthree\nfour\n").unwrap();
        edit["status"] = "completed".into();
        let turn = tracker.observe("chat", &root, &mut edit);
        assert_eq!(edit["changes"][0]["added"], 2);
        assert_eq!(edit["changes"][0]["removed"], 1);
        assert_eq!(patch_change("中文.ts", &turn[0].diff).unwrap()["added"], 3);
        let mut failed = json!({"id":"failed","tool":"Edit","arguments":{"file_path":"中文.ts"},"status":"inProgress"});
        tracker.observe("chat", &root, &mut failed);
        failed["status"] = "failed".into();
        tracker.observe("chat", &root, &mut failed);
        assert_eq!(failed["changes"][0]["added"], 0);
        assert!(tracker.operations.is_empty());
        tracker.finish_session("chat");
        assert!(tracker.baselines.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn patches_and_snapshots_do_not_guess_from_arguments_or_cross_workspaces() {
        let root =
            std::env::temp_dir().join(format!("supercode-edit-safety-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut tracker = Tracker::default();
        let mut item = json!({"id":"bad","arguments":{"file_path":"../outside.ts","content":"unwritten"},"status":"inProgress"});
        assert!(tracker.observe("chat", &root, &mut item).is_empty());
        assert!(tracker.operations.is_empty());
        let patch = "--- a/a\n+++ b/a\n@@ -1 +1 @@\n---old-code\n+++new-code\n";
        let row = patch_change("a", patch).unwrap();
        assert_eq!(row["added"], 1);
        assert_eq!(row["removed"], 1);
        assert!(patch_change("a", "+unverified argument").is_none());
        std::fs::write(root.join("binary"), [0xff, 0xfe]).unwrap();
        item["arguments"]["file_path"] = "binary".into();
        tracker.observe("chat", &root, &mut item);
        assert!(item["changeNote"].as_str().unwrap().contains("UTF-8"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_acp_diff_and_pi_patch_keep_their_exact_operation_counts() {
        let mut acp = json!({"contentItems":[{"type":"diff","path":"a.ts","oldText":"old\n","newText":"new\nextra\n"}]});
        normalize_native(&mut acp);
        assert_eq!(acp["changeSummary"][0]["added"], 2);
        assert_eq!(acp["changeSummary"][0]["removed"], 1);
        let mut pi = json!({"changes":[patch_change("a.ts","@@ -1 +1 @@\n-old\n+new\n").unwrap()]});
        normalize_native(&mut pi);
        assert_eq!(pi["changeSummary"][0]["added"], 1);
    }
}
