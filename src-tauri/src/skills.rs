use crate::client_features::read_utf8;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

const MAX_SKILLS: usize = 512;
const MAX_DIRS: usize = 4096;
pub const MAX_SKILL_BYTES: u64 = 128 * 1024;

#[derive(Clone)]
pub struct SkillRoot {
    pub path: PathBuf,
    pub label: String,
    pub namespace: Option<String>,
}
impl SkillRoot {
    fn new(path: PathBuf, label: String) -> Self {
        Self {
            path,
            label,
            namespace: None,
        }
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub source: String,
    pub sources: Vec<String>,
    pub namespace: Option<String>,
    pub error: Option<String>,
}
#[derive(Default, Serialize)]
pub struct Catalog {
    pub skills: Vec<Skill>,
    pub warnings: Vec<String>,
}

fn path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}
fn scalar(value: &str) -> String {
    let value = value.trim();
    if value.starts_with('"') {
        serde_json::from_str::<String>(value).unwrap_or_else(|_| value.trim_matches('"').into())
    } else if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        value[1..value.len() - 1].replace("''", "'")
    } else {
        value.split(" #").next().unwrap_or(value).trim().into()
    }
}
fn metadata(content: &str, fallback: &str) -> (String, String) {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (fallback.into(), String::new());
    }
    let front: Vec<_> = lines.take_while(|line| line.trim() != "---").collect();
    let field = |key: &str| -> Option<String> {
        let at = front
            .iter()
            .position(|line| line.starts_with(&format!("{key}:")))?;
        let value = front[at][key.len() + 1..].trim();
        if value.is_empty() || matches!(value, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
            let parts: Vec<_> = front[at + 1..]
                .iter()
                .take_while(|line| line.is_empty() || line.starts_with([' ', '\t']))
                .map(|line| line.trim())
                .collect();
            Some(
                parts
                    .join(if value.starts_with('|') { "\n" } else { " " })
                    .trim()
                    .into(),
            )
        } else {
            Some(scalar(value))
        }
    };
    (
        field("name")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback.into()),
        field("description").unwrap_or_default(),
    )
}

/// Scan metadata only. Skill contents and supporting files are loaded when selected.
/// Follow junctions/symlinks while bounding traversal and preventing directory cycles.
pub fn discover(roots: &[SkillRoot]) -> Catalog {
    let mut catalog = Catalog::default();
    let mut files = HashMap::<String, usize>::new();
    let mut copies = HashMap::<(String, u64), usize>::new();
    let mut visited_count = 0;
    for root in roots {
        let mut visited = HashSet::new();
        let mut pending = vec![(root.path.clone(), 0)];
        while let Some((dir, depth)) = pending.pop() {
            if catalog.skills.len() >= MAX_SKILLS || visited_count >= MAX_DIRS {
                catalog
                    .warnings
                    .push("技能扫描达到数量上限，部分目录尚未读取".into());
                return catalog;
            }
            let Ok(canonical_dir) = dir.canonicalize() else {
                continue;
            };
            if !canonical_dir.is_dir() || !visited.insert(path_key(&canonical_dir)) {
                continue;
            }
            visited_count += 1;
            let path = dir.join("SKILL.md");
            if path.is_file() {
                let canonical = path.canonicalize().unwrap_or(path.clone());
                let key = path_key(&canonical);
                if let Some(index) = files.get(&key) {
                    let skill = &mut catalog.skills[*index];
                    if !skill.sources.contains(&root.label) {
                        skill.sources.push(root.label.clone());
                    }
                    continue;
                }
                let fallback = dir.file_name().unwrap_or_default().to_string_lossy();
                let content = read_utf8(&canonical, MAX_SKILL_BYTES);
                let (name, description) = content
                    .as_ref()
                    .map(|text| metadata(text, &fallback))
                    .unwrap_or_else(|_| (fallback.into(), String::new()));
                let duplicate = content.as_ref().ok().and_then(|text| {
                    let mut hash = std::collections::hash_map::DefaultHasher::new();
                    text.hash(&mut hash);
                    let identity = (name.clone(), hash.finish());
                    if let Some(index) = copies.get(&identity) {
                        Some(*index)
                    } else {
                        copies.insert(identity, catalog.skills.len());
                        None
                    }
                });
                if let Some(index) = duplicate {
                    files.insert(key, index);
                    let skill = &mut catalog.skills[index];
                    if !skill.sources.contains(&root.label) {
                        skill.sources.push(root.label.clone());
                    }
                    continue;
                }
                files.insert(key, catalog.skills.len());
                catalog.skills.push(Skill {
                    name,
                    description,
                    path: canonical,
                    source: root.label.clone(),
                    sources: vec![root.label.clone()],
                    namespace: root.namespace.clone(),
                    error: content.err(),
                });
                // Do not walk a skill's scripts, references or vendored dependencies.
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                catalog
                    .warnings
                    .push(format!("无法读取技能目录：{}", dir.display()));
                continue;
            };
            let mut dirs: Vec<_> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_dir()
                        && !matches!(
                            path.file_name().and_then(|v| v.to_str()),
                            Some("node_modules" | ".git" | "target" | "dist")
                        )
                })
                .collect();
            if depth >= 8 {
                if !dirs.is_empty() {
                    catalog
                        .warnings
                        .push(format!("技能目录层级过深：{}", dir.display()));
                }
                continue;
            }
            dirs.sort();
            pending.extend(dirs.into_iter().rev().map(|path| (path, depth + 1)));
        }
    }
    catalog
        .skills
        .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    catalog
}

pub fn read_selected(path: &Path, roots: &[SkillRoot]) -> Result<String, String> {
    let canonical = path.canonicalize().map_err(|_| "技能文件不存在")?;
    if canonical.file_name().is_none_or(|v| v != "SKILL.md")
        || !discover(roots)
            .skills
            .iter()
            .any(|s| path_key(&s.path) == path_key(&canonical))
    {
        return Err("路径不在已发现技能目录内".into());
    }
    read_utf8(&canonical, MAX_SKILL_BYTES)
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    read_utf8(path, 2 * 1024 * 1024)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}
fn valid_component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', ':'])
}
fn version_parts(value: &str) -> Vec<u64> {
    value.split('.').map(|v| v.parse().unwrap_or(0)).collect()
}
fn codex_plugin_roots(home: &Path, roots: &mut Vec<SkillRoot>) {
    let Some(config) = read_utf8(&home.join("config.toml"), 2 * 1024 * 1024)
        .ok()
        .and_then(|s| s.parse::<toml::Value>().ok())
    else {
        return;
    };
    let Some(plugins) = config.get("plugins").and_then(toml::Value::as_table) else {
        return;
    };
    for (id, config) in plugins {
        if config.get("enabled").and_then(toml::Value::as_bool) != Some(true) {
            continue;
        }
        let Some((name, marketplace)) = id.rsplit_once('@') else {
            continue;
        };
        if !valid_component(name) || !valid_component(marketplace) {
            continue;
        }
        let cache = home.join("plugins/cache").join(marketplace).join(name);
        let Ok(entries) = std::fs::read_dir(cache) else {
            continue;
        };
        let mut versions: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
        versions.sort_by_key(|e| version_parts(&e.file_name().to_string_lossy()));
        if let Some(version) = versions.last() {
            roots.push(SkillRoot {
                path: version.path().join("skills"),
                label: format!("插件 · {name}（Codex）"),
                namespace: Some(name.into()),
            });
        }
    }
}
fn claude_plugin_roots(home: &Path, project: Option<&Path>, roots: &mut Vec<SkillRoot>) {
    let Some(installed) = read_json(&home.join("plugins/installed_plugins.json")) else {
        return;
    };
    let settings = read_json(&home.join("settings.json")).unwrap_or_default();
    let Some(plugins) = installed["plugins"].as_object() else {
        return;
    };
    for (id, installs) in plugins {
        if settings["enabledPlugins"][id] == false {
            continue;
        }
        let name = id.split('@').next().unwrap_or(id);
        for install in installs.as_array().into_iter().flatten() {
            if install["scope"] != "user"
                && !install["projectPath"].as_str().is_some_and(|p| {
                    project.is_some_and(|project| path_key(Path::new(p)) == path_key(project))
                })
            {
                continue;
            }
            if let Some(path) = install["installPath"].as_str() {
                roots.push(SkillRoot {
                    path: Path::new(path).join("skills"),
                    label: format!("插件 · {name}（Claude）"),
                    namespace: Some(name.into()),
                });
            }
        }
    }
}
pub fn roots(
    app_data: &Path,
    home: Option<&Path>,
    codex_home: Option<&Path>,
    claude_home: Option<&Path>,
    project: Option<&Path>,
) -> Vec<SkillRoot> {
    let mut roots = vec![];
    if let Some(project) = project {
        for (level, ancestor) in project.ancestors().enumerate() {
            for sub in [".agents/skills", ".codex/skills", ".claude/skills"] {
                roots.push(SkillRoot::new(
                    ancestor.join(sub),
                    format!("{} · {sub}", if level == 0 { "项目" } else { "上级项目" }),
                ));
            }
        }
    }
    roots.push(SkillRoot::new(app_data.join("skills"), "SuperCode".into()));
    if let Some(home) = home {
        roots.push(SkillRoot::new(
            home.join(".agents/skills"),
            "本机 · .agents".into(),
        ));
    }
    let codex = codex_home
        .map(Path::to_path_buf)
        .or_else(|| home.map(|p| p.join(".codex")));
    let claude = claude_home
        .map(Path::to_path_buf)
        .or_else(|| home.map(|p| p.join(".claude")));
    if let Some(codex) = codex {
        roots.push(SkillRoot::new(codex.join("skills"), "本机 · Codex".into()));
        codex_plugin_roots(&codex, &mut roots);
    }
    if let Some(claude) = claude {
        roots.push(SkillRoot::new(
            claude.join("skills"),
            "本机 · Claude".into(),
        ));
        claude_plugin_roots(&claude, project, &mut roots);
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let path = std::env::temp_dir().join(format!("supercode-skills-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn write(root: &Path, sub: &str, text: &str) -> PathBuf {
        let dir = root.join(sub);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        std::fs::write(&path, text).unwrap();
        path
    }
    #[test]
    fn nested_skills_and_identical_copies_are_discovered_with_sources() {
        let root = fixture();
        let text = "---\nname: recall\ndescription: 恢复状态\n---\n完整正文";
        write(&root, "first/.system/recall", text);
        write(&root, "second/recall", text);
        write(&root, "second/save", "---\nname: save\n---\n保存状态");
        let roots = [
            SkillRoot::new(root.join("first"), "Codex".into()),
            SkillRoot::new(root.join("second"), "Claude".into()),
        ];
        let catalog = discover(&roots);
        assert_eq!(catalog.skills.len(), 2);
        assert_eq!(catalog.skills[0].sources, vec!["Codex", "Claude"]);
        assert_eq!(
            read_selected(&catalog.skills[0].path, &roots).unwrap(),
            text
        );
        assert!(read_selected(&write(&root, "outside", "outside"), &roots).is_err());
    }
    #[test]
    fn different_same_named_skills_and_invalid_encoding_are_not_hidden() {
        let root = fixture();
        write(&root, "one", "---\nname: save\n---\none");
        write(&root, "two", "---\nname: save\n---\ntwo");
        let invalid = write(&root, "bad", "bad");
        std::fs::write(&invalid, [0xff, 0xfe]).unwrap();
        let catalog = discover(&[SkillRoot::new(root, "本机".into())]);
        assert_eq!(catalog.skills.len(), 3);
        assert!(catalog
            .skills
            .iter()
            .find(|s| s.name == "bad")
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("UTF-8"));
    }
    #[test]
    fn frontmatter_parses_multiline_fields_without_reading_body_as_metadata() {
        assert_eq!(
            metadata(
                "---\nname: 'recall'\ndescription: >-\n  第一行\n  第二行\n---\nname: wrong",
                "fallback"
            ),
            ("recall".into(), "第一行 第二行".into())
        );
        assert_eq!(
            metadata("body\nname: wrong", "fallback"),
            ("fallback".into(), String::new())
        );
        assert_eq!(metadata("---\nname: '\n---", "fallback").0, "'");
        assert_eq!(
            metadata("---\nname: example\ndescription:\n  first line\n  second line\nlicense: MIT\n---\nbody", "fallback").1,
            "first line second line"
        );
    }
    #[test]
    fn configured_homes_ancestor_skills_and_enabled_plugin_versions_are_supported() {
        let root = fixture();
        let codex = root.join("custom-codex");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("config.toml"),"[plugins.\"active@market\"]\nenabled=true\n[plugins.\"disabled@market\"]\nenabled=false\n").unwrap();
        write(
            &codex,
            "plugins/cache/market/active/1.9.0/skills/old",
            "old",
        );
        write(
            &codex,
            "plugins/cache/market/active/1.10.0/skills/new",
            "new",
        );
        write(
            &codex,
            "plugins/cache/market/disabled/1/skills/disabled",
            "disabled",
        );
        write(
            &root,
            "project/.agents/skills/save",
            "---\nname: save\n---\nfull",
        );
        std::fs::create_dir_all(root.join("project/child")).unwrap();
        let roots = roots(
            &root.join("app"),
            None,
            Some(&codex),
            None,
            Some(&root.join("project/child")),
        );
        let names: Vec<_> = discover(&roots)
            .skills
            .into_iter()
            .filter(|s| s.path.starts_with(root.canonicalize().unwrap()))
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["new", "save"]);
    }
}
