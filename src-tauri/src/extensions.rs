use crate::{client_features::read_utf8, AppState};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

// Inventory reads never start agents or MCP processes. Credentials stay in Rust.
const MAX_FILE: u64 = 2 * 1024 * 1024;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub id: String,
    pub native_id: String,
    pub agent: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub path: PathBuf,
    pub installed: bool,
    pub enabled: bool,
    pub source: String,
    pub components: Vec<String>,
    pub icon: Option<String>,
    pub limitation: Option<String>,
    #[serde(skip)]
    manifest: Value,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mcp {
    pub id: String,
    pub name: String,
    pub agent: String,
    pub source: String,
    pub endpoint: String,
    pub transport: String,
    pub enabled: bool,
    pub plugin_id: Option<String>,
    pub limitation: Option<String>,
    #[serde(skip)]
    config: Value,
    #[serde(skip)]
    native_name: String,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub plugins: Vec<Plugin>,
    pub mcp: Vec<Mcp>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Import {
    id: String,
    agent: String,
    path: PathBuf,
}
#[derive(Default)]
struct Choices {
    states: BTreeMap<String, bool>,
    imports: Vec<Import>,
    load_mcp: bool,
}
struct Roots {
    codex: PathBuf,
    claude: PathBuf,
    home: PathBuf,
    project: Option<PathBuf>,
}
impl Roots {
    fn for_app(app: &AppHandle, project_id: Option<&str>) -> Result<Self, String> {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .ok_or("无法找到用户目录")?;
        Ok(Self {
            codex: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex")),
            claude: std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude")),
            project: project_id
                .filter(|id| !id.is_empty())
                .map(|id| app.state::<AppState>().store.project(id))
                .transpose()?
                .map(|p| PathBuf::from(p.path)),
            home,
        })
    }
}
fn read_choices(app: &AppHandle) -> Result<Choices, String> {
    let state = app.state::<AppState>();
    Ok(Choices {
        states: serde_json::from_str(
            &state
                .store
                .setting("extension_states")?
                .unwrap_or_else(|| "{}".into()),
        )
        .map_err(|_| "扩展开关配置无效")?,
        imports: serde_json::from_str(
            &state
                .store
                .setting("extension_imports")?
                .unwrap_or_else(|| "[]".into()),
        )
        .map_err(|_| "插件目录配置无效")?,
        load_mcp: state.store.load_mcp()?,
    })
}
fn read(path: &Path, warnings: &mut Vec<String>) -> Option<Value> {
    if !path.exists() {
        return None;
    }
    match read_utf8(path, MAX_FILE).and_then(|text| {
        if path.extension().is_some_and(|v| v == "toml") {
            text.parse::<toml::Value>()
                .map_err(|_| format!("{}：TOML 格式无效", path.display()))
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
        } else {
            serde_json::from_str(&text).map_err(|_| format!("{}：JSON 格式无效", path.display()))
        }
    }) {
        Ok(v) => Some(v),
        Err(e) => {
            warnings.push(format!("{}：{e}", path.display()));
            None
        }
    }
}
fn component(s: &str) -> bool {
    !s.is_empty() && !matches!(s, "." | "..") && !s.contains(['/', '\\', ':'])
}
fn dirs(path: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .take(256)
        .map(|e| e.path())
        .collect();
    paths.sort();
    paths
}
fn latest(path: &Path) -> Option<PathBuf> {
    let mut versions = dirs(path);
    versions.sort_by_key(|p| {
        (
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .split('.')
                .map(|v| v.parse::<u64>().unwrap_or(0))
                .collect::<Vec<_>>(),
            p.clone(),
        )
    });
    versions.pop()
}
fn child(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let path = root.join(relative).canonicalize().ok()?;
    path.starts_with(root).then_some(path)
}
fn manifest(root: &Path, agent: &str, warnings: &mut Vec<String>) -> Option<Value> {
    for path in [
        "plugin.json",
        if agent == "codex" {
            ".codex-plugin/plugin.json"
        } else {
            ".claude-plugin/plugin.json"
        },
    ] {
        if let Some(file) = child(root, path) {
            return read(&file, warnings);
        }
    }
    None
}
fn metadata<'a>(manifest: &'a Value) -> &'a Value {
    manifest
        .pointer("/extensions/com.openai")
        .unwrap_or(manifest)
}
fn bundle(root: &Path, manifest: &Value, warnings: &mut Vec<String>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for file in [".mcp.json", "mcp.json"] {
        if let Some(file) = child(root, file) {
            if let Some(v) = read(&file, warnings) {
                merge_servers(&mut out, &v);
            }
        }
    }
    fn add(
        root: &Path,
        value: &Value,
        out: &mut BTreeMap<String, Value>,
        warnings: &mut Vec<String>,
    ) {
        match value {
            Value::String(path) => {
                if let Some(file) = child(root, path) {
                    if let Some(v) = read(&file, warnings) {
                        merge_servers(out, &v);
                    }
                } else {
                    warnings.push(format!(
                        "插件 MCP 文件不存在或超出插件目录：{}",
                        root.display()
                    ));
                }
            }
            Value::Array(values) => {
                for value in values.iter().take(32) {
                    add(root, value, out, warnings);
                }
            }
            Value::Object(_) => merge_servers(out, value),
            _ => (),
        }
    }
    add(root, &metadata(manifest)["mcpServers"], &mut out, warnings);
    out
}
fn merge_servers(out: &mut BTreeMap<String, Value>, value: &Value) {
    if let Some(map) = value.get("mcpServers").unwrap_or(value).as_object() {
        out.extend(
            map.iter()
                .take(128)
                .filter(|(_, v)| v.is_object())
                .map(|(k, v)| (k.clone(), v.clone())),
        );
    }
}
fn icon(root: &Path, manifest: &Value) -> Option<String> {
    let path = metadata(manifest)["interface"]["logo"]
        .as_str()
        .or_else(|| metadata(manifest)["interface"]["composerIcon"].as_str())?;
    let file = child(root, path)?;
    if std::fs::metadata(&file).ok()?.len() > 256 * 1024 {
        return None;
    }
    let mime = match file.extension()?.to_str()? {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return None,
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(std::fs::read(file).ok()?)
    ))
}
fn limitation(config: &Value) -> Option<String> {
    let text = config.to_string();
    if [
        "CODEX_APP_TOOLS_PIPE_PATH",
        "CODEX_BROWSER",
        "CODEX_ELECTRON",
        "CODEX_COMPUTER",
        "SKY_CUA_NATIVE_PIPE_DIRECTORY",
        "BROWSER_USE_CODEX_APP_BUILD_FLAVOR",
    ]
    .iter()
    .any(|v| text.contains(v))
    {
        Some("需要 Codex 客户端提供的连接".into())
    } else if config["type"] == "sdk" {
        Some("需要对应的应用宿主".into())
    } else {
        None
    }
}
fn plugin(
    root: PathBuf,
    agent: &str,
    native_id: &str,
    id: String,
    installed: bool,
    enabled: bool,
    source: &str,
    choices: &Choices,
    warnings: &mut Vec<String>,
) -> Option<Plugin> {
    let manifest = match manifest(&root, agent, warnings) {
        Some(value) => value,
        None if agent == "claude"
            && !root.join("plugin.json").exists()
            && !root.join(".claude-plugin/plugin.json").exists()
            && [
                "skills",
                "commands",
                ".mcp.json",
                ".lsp.json",
                "hooks/hooks.json",
            ]
            .iter()
            .any(|path| root.join(path).exists()) =>
        {
            // Claude supports manifest-free plugins with standard component paths.
            json!({"name":native_id.split('@').next().unwrap_or(native_id)})
        }
        None => {
            if installed {
                warnings.push(format!(
                    "插件 {native_id} 的本机文件不完整：缺少有效清单或组件"
                ));
            }
            return None;
        }
    };
    let meta = metadata(&manifest);
    let servers = bundle(&root, &manifest, warnings);
    let mut components = vec![];
    if root.join("skills").is_dir() || !meta["skills"].is_null() {
        components.push("技能".into());
    }
    if !servers.is_empty() {
        components.push("MCP".into());
    }
    if root.join("commands").is_dir() {
        components.push("命令".into());
    }
    if root.join(".lsp.json").is_file() || !meta["lspServers"].is_null() {
        components.push("LSP".into());
    }
    if root.join("hooks/hooks.json").is_file() || !meta["hooks"].is_null() {
        components.push("Hooks".into());
    }
    let limit = if components == ["Hooks"] {
        Some("自动执行的 Hooks 在 SuperCode 中关闭".into())
    } else if !servers.is_empty()
        && servers.values().all(|v| limitation(v).is_some())
        && !components.iter().any(|v| v == "技能")
    {
        Some("需要 Codex 客户端提供的连接".into())
    } else {
        None
    };
    Some(Plugin {
        id: id.clone(),
        native_id: native_id.into(),
        agent: agent.into(),
        name: meta["interface"]["displayName"]
            .as_str()
            .or_else(|| manifest["name"].as_str())
            .unwrap_or(native_id.split('@').next().unwrap_or(native_id))
            .into(),
        description: meta["interface"]["shortDescription"]
            .as_str()
            .or_else(|| manifest["description"].as_str())
            .unwrap_or("")
            .into(),
        version: manifest["version"].as_str().unwrap_or("").into(),
        icon: icon(&root, &manifest),
        path: root,
        installed,
        enabled: installed
            && choices.states.get(&id).copied().unwrap_or(enabled)
            && limit.is_none(),
        source: source.into(),
        components,
        limitation: limit,
        manifest,
    })
}
fn mcp(
    name: &str,
    agent: &str,
    source: &str,
    config: Value,
    id: String,
    default: bool,
    plugin_id: Option<String>,
    choices: &Choices,
) -> Mcp {
    let limit = limitation(&config);
    let url = config["url"].as_str();
    let endpoint = url
        .and_then(|u| reqwest::Url::parse(u).ok())
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_else(|| {
            config["command"]
                .as_str()
                .and_then(|p| Path::new(p).file_name())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    Mcp {
        id: id.clone(),
        name: name.into(),
        agent: agent.into(),
        source: source.into(),
        endpoint,
        transport: if url.is_some() { "HTTP" } else { "stdio" }.into(),
        enabled: limit.is_none() && choices.states.get(&id).copied().unwrap_or(default),
        plugin_id,
        limitation: limit,
        config,
        native_name: name.into(),
    }
}
fn discover(roots: &Roots, choices: &Choices, browse: bool) -> Catalog {
    let mut out = Catalog::default();
    let codex = read(&roots.codex.join("config.toml"), &mut out.warnings).unwrap_or_default();
    let mut plugins = codex["plugins"].as_object().cloned().unwrap_or_default();
    if let Some(project) = &roots.project {
        if let Some(config) = read(&project.join(".codex/config.toml"), &mut out.warnings) {
            if let Some(p) = config["plugins"].as_object() {
                plugins.extend(p.clone());
            }
        }
    }
    for (native, config) in &plugins {
        let Some((name, market)) = native
            .rsplit_once('@')
            .filter(|(n, m)| component(n) && component(m))
        else {
            continue;
        };
        if let Some(root) = latest(&roots.codex.join("plugins/cache").join(market).join(name)) {
            if let Some(p) = plugin(
                root,
                "codex",
                native,
                format!("codex:plugin:{native}"),
                true,
                config["enabled"] == true,
                market,
                choices,
                &mut out.warnings,
            ) {
                out.plugins.push(p);
            }
        } else {
            out.warnings
                .push(format!("Codex 插件 {native} 的本机文件不存在"));
        }
    }
    let settings = read(&roots.claude.join("settings.json"), &mut out.warnings).unwrap_or_default();
    let mut enabled = settings["enabledPlugins"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(project) = &roots.project {
        for sub in [".claude/settings.json", ".claude/settings.local.json"] {
            if let Some(v) = read(&project.join(sub), &mut out.warnings) {
                if let Some(map) = v["enabledPlugins"].as_object() {
                    enabled.extend(map.clone());
                }
            }
        }
    }
    let installed = read(
        &roots.claude.join("plugins/installed_plugins.json"),
        &mut out.warnings,
    )
    .unwrap_or_default();
    if let Some(map) = installed["plugins"].as_object() {
        for (native, installs) in map {
            for install in installs.as_array().into_iter().flatten() {
                if install["scope"] != "user"
                    && !install["projectPath"].as_str().is_some_and(|v| {
                        roots
                            .project
                            .as_deref()
                            .is_some_and(|p| path_eq(p, Path::new(v)))
                    })
                {
                    continue;
                }
                if let Some(path) = install["installPath"].as_str() {
                    if let Some(p) = plugin(
                        PathBuf::from(path),
                        "claude",
                        native,
                        format!("claude:plugin:{native}"),
                        true,
                        enabled.get(native) != Some(&Value::Bool(false)),
                        native.rsplit_once('@').map(|(_, v)| v).unwrap_or("本机"),
                        choices,
                        &mut out.warnings,
                    ) {
                        if !out.plugins.iter().any(|v| v.id == p.id) {
                            out.plugins.push(p);
                        }
                    }
                }
            }
        }
    }
    for import in &choices.imports {
        if let Some(p) = plugin(
            import.path.clone(),
            &import.agent,
            &import.id,
            import.id.clone(),
            true,
            true,
            "SuperCode 导入",
            choices,
            &mut out.warnings,
        ) {
            out.plugins.push(p);
        }
    }
    // Plugin MCPs are distinct from standalone MCPs, with their own on/off state.
    for p in &out.plugins {
        for (name, config) in bundle(&p.path, &p.manifest, &mut out.warnings) {
            let mut s = mcp(
                &name,
                &p.agent,
                &p.name,
                config.clone(),
                format!("{}:mcp:{name}", p.id),
                p.enabled && config["enabled"] != false,
                Some(p.id.clone()),
                choices,
            );
            s.enabled &= p.enabled;
            if p.agent == "claude" {
                s.native_name = format!(
                    "plugin:{}:{name}",
                    p.native_id.split('@').next().unwrap_or(&p.native_id)
                );
            }
            out.mcp.push(s);
        }
    }
    if let Some(map) = codex["mcp_servers"].as_object() {
        for (name, config) in map {
            out.mcp.push(mcp(
                name,
                "codex",
                "Codex 本机配置",
                config.clone(),
                format!("codex:mcp:{name}"),
                choices.load_mcp && config["enabled"] != false,
                None,
                choices,
            ));
        }
    }
    let claude_path = if roots.claude == roots.home.join(".claude") {
        roots.home.join(".claude.json")
    } else {
        roots.claude.join(".claude.json")
    };
    let claude = read(&claude_path, &mut out.warnings).unwrap_or_default();
    let mut native = BTreeMap::new();
    merge_servers(&mut native, &claude["mcpServers"]);
    if let Some(project) = &roots.project {
        if let Some(v) = read(&project.join(".mcp.json"), &mut out.warnings) {
            merge_servers(&mut native, &v);
        }
        if let Some(projects) = claude["projects"].as_object() {
            for (path, value) in projects {
                if path_eq(project, Path::new(path)) {
                    merge_servers(&mut native, &value["mcpServers"]);
                }
            }
        }
    }
    for (name, config) in native {
        out.mcp.push(mcp(
            &name,
            "claude",
            "Claude Code 本机配置",
            config,
            format!("claude:mcp:{name}"),
            false,
            None,
            choices,
        ));
    }
    if browse {
        let ids: HashSet<_> = out
            .plugins
            .iter()
            .map(|p| (p.agent.clone(), p.native_id.clone()))
            .collect();
        for (agent, home) in [("codex", &roots.codex), ("claude", &roots.claude)] {
            for market in dirs(&home.join("plugins/cache")) {
                for name in dirs(&market) {
                    let native = format!(
                        "{}@{}",
                        name.file_name().unwrap_or_default().to_string_lossy(),
                        market.file_name().unwrap_or_default().to_string_lossy()
                    );
                    if ids.contains(&(agent.into(), native.clone())) {
                        continue;
                    }
                    if let Some(root) = latest(&name) {
                        if let Some(p) = plugin(
                            root,
                            agent,
                            &native,
                            format!("{agent}:cache:{native}"),
                            false,
                            false,
                            &market.file_name().unwrap_or_default().to_string_lossy(),
                            choices,
                            &mut out.warnings,
                        ) {
                            out.plugins.push(p);
                        }
                    }
                }
            }
        }
    }
    out.plugins
        .sort_by_key(|p| (!p.installed, p.agent.clone(), p.name.to_lowercase()));
    out.mcp
        .sort_by_key(|p| (p.agent.clone(), p.name.to_lowercase()));
    out.warnings.sort();
    out.warnings.dedup();
    out
}
fn path_eq(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().replace('\\', "/").to_lowercase()
        == b.to_string_lossy().replace('\\', "/").to_lowercase()
}
#[tauri::command]
pub fn list_extensions(
    app: AppHandle,
    project_id: Option<String>,
    browse: Option<bool>,
) -> Result<Catalog, String> {
    Ok(discover(
        &Roots::for_app(&app, project_id.as_deref())?,
        &read_choices(&app)?,
        browse.unwrap_or(false),
    ))
}
#[tauri::command]
pub async fn set_extension_enabled(
    app: AppHandle,
    project_id: Option<String>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    if app.state::<AppState>().store.running()? > 0 {
        return Err("任务结束后才能更改插件和 MCP".into());
    }
    let mut choices = read_choices(&app)?;
    let catalog = discover(
        &Roots::for_app(&app, project_id.as_deref())?,
        &choices,
        false,
    );
    if !catalog
        .plugins
        .iter()
        .any(|p| p.id == id && p.installed && p.limitation.is_none())
        && !catalog
            .mcp
            .iter()
            .any(|m| m.id == id && m.limitation.is_none())
    {
        return Err("扩展不存在或需要官方客户端提供连接".into());
    }
    choices.states.insert(id, enabled);
    app.state::<AppState>().store.set_setting(
        "extension_states",
        &serde_json::to_string(&choices.states).map_err(|e| e.to_string())?,
    )?;
    app.state::<AppState>()
        .runtime
        .clear_for_agent_switch()
        .await;
    Ok(())
}
#[tauri::command]
pub async fn import_extension(app: AppHandle, path: String, agent: String) -> Result<(), String> {
    if app.state::<AppState>().store.running()? > 0 {
        return Err("任务结束后才能导入插件".into());
    }
    if !matches!(agent.as_str(), "codex" | "claude") {
        return Err("请选择 Codex 或 Claude Code".into());
    }
    let path = Path::new(&path)
        .canonicalize()
        .map_err(|_| "插件目录不存在")?;
    let mut warnings = vec![];
    let parsed = manifest(&path, &agent, &mut warnings).ok_or_else(|| {
        warnings
            .first()
            .cloned()
            .unwrap_or("目录内没有对应 Agent 的 plugin.json".into())
    })?;
    if parsed["name"].as_str().is_none_or(|v| !component(v)) {
        return Err("插件名称无效".into());
    }
    let mut choices = read_choices(&app)?;
    let roots = Roots::for_app(&app, None)?;
    let existing = discover(&roots, &choices, true)
        .plugins
        .into_iter()
        .find(|p| p.agent == agent && path_eq(&p.path, &path));
    if let Some(p) = existing.as_ref().filter(|p| p.installed) {
        choices.states.insert(p.id.clone(), true);
    } else {
        if choices.imports.len() >= 64 {
            return Err("最多导入 64 个插件".into());
        }
        choices.imports.push(Import {
            id: format!("import:{}", uuid::Uuid::new_v4()),
            agent,
            path,
        });
    }
    app.state::<AppState>().store.set_setting(
        "extension_imports",
        &serde_json::to_string(&choices.imports).map_err(|e| e.to_string())?,
    )?;
    app.state::<AppState>().store.set_setting(
        "extension_states",
        &serde_json::to_string(&choices.states).map_err(|e| e.to_string())?,
    )?;
    app.state::<AppState>()
        .runtime
        .clear_for_agent_switch()
        .await;
    Ok(())
}
#[tauri::command]
pub fn open_extension_folder(
    app: AppHandle,
    project_id: Option<String>,
    id: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let catalog = discover(
        &Roots::for_app(&app, project_id.as_deref())?,
        &read_choices(&app)?,
        true,
    );
    let p = catalog
        .plugins
        .iter()
        .find(|p| p.id == id)
        .ok_or("插件不存在")?;
    app.opener()
        .open_path(p.path.to_string_lossy().as_ref(), None::<&str>)
        .map_err(|e| e.to_string())
}
pub fn codex_overrides(app: &AppHandle, project: Option<&Path>) -> Result<Vec<String>, String> {
    let mut roots = Roots::for_app(app, None)?;
    roots.project = project.map(Path::to_owned);
    let catalog = discover(&roots, &read_choices(app)?, false);
    codex_overrides_for(&catalog)
}
fn codex_overrides_for(catalog: &Catalog) -> Result<Vec<String>, String> {
    let mut out = vec![];
    for p in &catalog.plugins {
        if p.agent == "codex" && !p.id.starts_with("import:") {
            out.push(format!("plugins.{}.enabled={}", p.native_id, p.enabled));
        }
    }
    for m in catalog.mcp.iter().filter(|m| m.agent == "codex") {
        let prefix = if let Some(id) = &m.plugin_id {
            let p = catalog.plugins.iter().find(|p| &p.id == id).unwrap();
            if p.id.starts_with("import:") {
                let name = format!(
                    "supercode_plugin_{}_{}",
                    p.id.replace([':', '-'], "_"),
                    m.name.replace([':', '-', '.'], "_")
                );
                let prefix = format!("mcp_servers.{name}");
                // Only explicit transport fields, never plugin tool approval policies.
                if m.enabled {
                    for field in [
                        "command",
                        "args",
                        "env",
                        "cwd",
                        "url",
                        "http_headers",
                        "bearer_token_env_var",
                    ] {
                        if let Some(value) = m.config.get(field) {
                            let mut value = value.clone();
                            expand(&mut value, &p.path, None);
                            let value: toml::Value =
                                serde_json::from_value(value).map_err(|_| "插件 MCP 配置无效")?;
                            out.push(format!("{prefix}.{field}={value}"));
                        }
                    }
                }
                prefix
            } else {
                format!("plugins.{}.mcp_servers.{}", p.native_id, m.name)
            }
        } else {
            format!("mcp_servers.{}", m.name)
        };
        out.push(format!("{prefix}.enabled={}", m.enabled));
    }
    Ok(out)
}
fn expand(value: &mut Value, root: &Path, project: Option<&Path>) {
    match value {
        Value::String(s) => {
            *s = s
                .replace(
                    "${CLAUDE_PLUGIN_ROOT}",
                    &root.to_string_lossy().replace('\\', "/"),
                )
                .replace(
                    "${CODEX_PLUGIN_ROOT}",
                    &root.to_string_lossy().replace('\\', "/"),
                )
                .replace(
                    "${CLAUDE_PROJECT_DIR}",
                    &project.unwrap_or(root).to_string_lossy().replace('\\', "/"),
                );
        }
        Value::Array(v) => {
            for value in v {
                expand(value, root, project);
            }
        }
        Value::Object(v) => {
            for value in v.values_mut() {
                expand(value, root, project);
            }
        }
        _ => (),
    }
}
pub fn claude_config(
    app: &AppHandle,
    project: &Path,
) -> Result<(Value, Value, Vec<String>), String> {
    let mut roots = Roots::for_app(app, None)?;
    roots.project = Some(project.to_owned());
    let catalog = discover(&roots, &read_choices(app)?, false);
    Ok(claude_config_for(&catalog, project))
}
fn claude_config_for(catalog: &Catalog, project: &Path) -> (Value, Value, Vec<String>) {
    let mut map = serde_json::Map::new();
    let mut enabled = serde_json::Map::new();
    let mut args = vec![];
    for p in catalog.plugins.iter().filter(|p| p.agent == "claude") {
        if p.id.starts_with("import:") {
            if p.enabled {
                args.extend(["--plugin-dir".into(), p.path.to_string_lossy().into_owned()]);
            }
        } else {
            enabled.insert(p.native_id.clone(), json!(p.enabled));
        }
    }
    for m in catalog
        .mcp
        .iter()
        .filter(|m| m.agent == "claude" && m.enabled)
    {
        let mut config = m.config.clone();
        // Do not carry automatic approval policy from a plugin into SuperCode.
        if let Some(object) = config.as_object_mut() {
            for key in [
                "enabled",
                "default_tools_approval_mode",
                "tools",
                "omit_tools_from",
            ] {
                object.remove(key);
            }
        }
        if let Some(id) = &m.plugin_id {
            if let Some(p) = catalog.plugins.iter().find(|p| &p.id == id) {
                expand(&mut config, &p.path, Some(project));
            }
        }
        if config["type"].is_null() {
            config["type"] = json!(if config["url"].is_string() {
                "http"
            } else {
                "stdio"
            });
        }
        map.insert(m.native_name.clone(), config);
    }
    (
        json!({"mcpServers":map}),
        json!({"disableAllHooks":true,"enabledPlugins":enabled}),
        args,
    )
}
pub fn filter_skill_roots(
    app: &AppHandle,
    project_id: Option<&str>,
    roots: &mut Vec<crate::skills::SkillRoot>,
) -> Result<(), String> {
    let catalog = discover(
        &Roots::for_app(app, project_id)?,
        &read_choices(app)?,
        false,
    );
    for p in &catalog.plugins {
        if !p.enabled {
            roots.retain(|root| !root.path.starts_with(&p.path));
        } else if p.path.join("skills").is_dir()
            && !roots
                .iter()
                .any(|r| path_eq(&r.path, &p.path.join("skills")))
        {
            roots.push(crate::skills::SkillRoot {
                path: p.path.join("skills"),
                label: format!("插件 · {}（{}）", p.name, p.agent),
                namespace: Some(p.name.clone()),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Roots) {
        let path =
            std::env::temp_dir().join(format!("supercode-extensions-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let roots = Roots {
            codex: path.join(".codex"),
            claude: path.join(".claude"),
            home: path.clone(),
            project: Some(path.join("project")),
        };
        (path, roots)
    }
    fn write(root: &Path, path: &str, data: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }
    #[test]
    fn discovers_installed_plugins_and_redacts_mcp_credentials() {
        let (path, roots) = fixture();
        write(&path, ".codex/config.toml", "[plugins.\"docs@local\"]\nenabled=true\n[mcp_servers.search]\nurl='https://example.com/mcp?key=secret'\nhttp_headers={Authorization='secret'}\n");
        for version in ["1.9.0", "1.10.0"] {
            write(
                &path,
                &format!(".codex/plugins/cache/local/docs/{version}/.codex-plugin/plugin.json"),
                &format!(r#"{{"name":"docs","version":"{version}","skills":"./skills"}}"#),
            );
        }
        write(&path, ".claude/plugins/installed_plugins.json", &json!({"plugins":{"context@market":[{"scope":"user","installPath":path.join("context")}]}}).to_string());
        write(
            &path,
            "context/.claude-plugin/plugin.json",
            r#"{"name":"context"}"#,
        );
        write(
            &path,
            "context/.mcp.json",
            r#"{"mcpServers":{"context":{"type":"http","url":"https://docs.example.com/mcp","headers":{"Authorization":"secret"}}}}"#,
        );
        let result = discover(&roots, &Choices::default(), false);
        assert_eq!(result.plugins.len(), 2);
        assert_eq!(result.plugins[0].version, "");
        assert!(result.plugins.iter().any(|p| p.version == "1.10.0"));
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("Authorization"));
        assert!(
            !result
                .mcp
                .iter()
                .find(|m| m.name == "search")
                .unwrap()
                .enabled
        );
        assert!(
            result
                .mcp
                .iter()
                .find(|m| m.name == "context")
                .unwrap()
                .enabled
        );
        let mut choices = Choices::default();
        choices.states.insert("codex:mcp:search".into(), true);
        choices
            .states
            .insert("claude:plugin:context@market".into(), false);
        let result = discover(&roots, &choices, false);
        assert!(
            result
                .mcp
                .iter()
                .find(|m| m.name == "search")
                .unwrap()
                .enabled
        );
        assert!(
            !result
                .mcp
                .iter()
                .find(|m| m.name == "context")
                .unwrap()
                .enabled
        );
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn plugin_paths_are_contained_and_invalid_encoding_is_reported() {
        let (path, roots) = fixture();
        write(
            &path,
            ".codex/config.toml",
            "[plugins.\"x@local\"]\nenabled=true\n",
        );
        write(
            &path,
            ".codex/plugins/cache/local/x/1/.codex-plugin/plugin.json",
            r#"{"name":"x","mcpServers":"../../outside.json"}"#,
        );
        let result = discover(&roots, &Choices::default(), false);
        assert!(!result.warnings.is_empty());
        assert!(result.mcp.is_empty());
        std::fs::write(roots.codex.join("config.toml"), [0xff, 0xfe]).unwrap();
        let result = discover(&roots, &Choices::default(), false);
        assert!(result.warnings.iter().any(|v| v.contains("UTF-8")));
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn app_bound_servers_stay_unavailable_and_browse_does_not_install() {
        let (path, roots) = fixture();
        write(
            &path,
            ".codex/config.toml",
            "[plugins.\"app@local\"]\nenabled=true\n",
        );
        write(
            &path,
            ".codex/plugins/cache/local/app/1/.codex-plugin/plugin.json",
            r#"{"name":"app","mcpServers":"./.mcp.json"}"#,
        );
        write(
            &path,
            ".codex/plugins/cache/local/app/1/.mcp.json",
            r#"{"mcpServers":{"app":{"command":"node","env_vars":["CODEX_APP_TOOLS_PIPE_PATH"]}}}"#,
        );
        write(
            &path,
            ".codex/plugins/cache/local/extra/1/.codex-plugin/plugin.json",
            r#"{"name":"extra","skills":"./skills"}"#,
        );
        let result = discover(&roots, &Choices::default(), true);
        assert_eq!(result.plugins.len(), 2);
        assert!(!result.plugins[0].enabled);
        assert!(!result.mcp[0].enabled);
        assert!(limitation(&json!({"env":{"SKY_CUA_NATIVE_PIPE_DIRECTORY":"pipe"}})).is_some());
        assert!(
            !result
                .plugins
                .iter()
                .find(|p| p.name == "extra")
                .unwrap()
                .installed
        );
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn claude_manifest_free_components_are_discovered_and_incomplete_installs_reported() {
        let (path, roots) = fixture();
        write(
            &path,
            ".claude/plugins/installed_plugins.json",
            &json!({"plugins":{
                "lsp@market":[{"scope":"user","installPath":path.join("lsp")}],
                "broken@market":[{"scope":"user","installPath":path.join("broken")}]
            }})
            .to_string(),
        );
        write(
            &path,
            "lsp/.lsp.json",
            r#"{"rust":{"command":"rust-analyzer"}}"#,
        );
        write(&path, "broken/README.md", "Incomplete cached installation");
        let result = discover(&roots, &Choices::default(), false);
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(result.plugins[0].name, "lsp");
        assert_eq!(result.plugins[0].components, ["LSP"]);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("broken@market")));
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn codex_project_plugin_choices_reach_runtime_overrides() {
        let (path, roots) = fixture();
        write(
            &path,
            "project/.codex/config.toml",
            "[plugins.\"local@market\"]\nenabled=true\n",
        );
        write(
            &path,
            ".codex/plugins/cache/market/local/1/plugin.json",
            r#"{"name":"local","skills":"./skills"}"#,
        );
        let mut choices = Choices::default();
        choices
            .states
            .insert("codex:plugin:local@market".into(), false);
        let catalog = discover(&roots, &choices, false);
        assert_eq!(catalog.plugins.len(), 1);
        assert!(codex_overrides_for(&catalog)
            .unwrap()
            .contains(&"plugins.local@market.enabled=false".into()));
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn runtime_uses_individual_choices_without_changing_native_files_or_approvals() {
        let (path, roots) = fixture();
        let original = "[mcp_servers.search]\ncommand='node'\nargs=['a b']\n";
        write(&path, ".codex/config.toml", original);
        write(
            &path,
            "import/.claude-plugin/plugin.json",
            r#"{"name":"docs"}"#,
        );
        write(
            &path,
            "import/.mcp.json",
            r#"{"mcpServers":{"docs":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/server.js"],"default_tools_approval_mode":"approve"}}}"#,
        );
        let mut choices = Choices::default();
        choices.states.insert("codex:mcp:search".into(), true);
        choices.imports.push(Import {
            id: "import:docs".into(),
            agent: "claude".into(),
            path: path.join("import"),
        });
        let catalog = discover(&roots, &choices, false);
        assert!(codex_overrides_for(&catalog)
            .unwrap()
            .contains(&"mcp_servers.search.enabled=true".into()));
        let (mcp, settings, args) = claude_config_for(&catalog, &path);
        assert_eq!(settings["disableAllHooks"], true);
        assert_eq!(args[0], "--plugin-dir");
        let config = mcp["mcpServers"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap();
        assert!(config["default_tools_approval_mode"].is_null());
        assert!(!config["args"][0]
            .as_str()
            .unwrap()
            .contains("${CLAUDE_PLUGIN_ROOT}"));
        assert_eq!(
            std::fs::read_to_string(roots.codex.join("config.toml")).unwrap(),
            original
        );
        choices.states.insert("import:docs".into(), false);
        assert!(
            claude_config_for(&discover(&roots, &choices, false), &path).0["mcpServers"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(path).unwrap();
    }
}
