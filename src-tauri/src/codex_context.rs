//! API model metadata for Codex. Account/native routes retain Codex's own catalog.
use crate::session_config::Route;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Configuration {
    managed: bool,
    model: Option<String>,
    window: Option<u64>,
    effective_percent: u64,
    mapped: Vec<ModelWindow>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct ModelWindow {
    model: String,
    window: u64,
    percent: u64,
}
fn template() -> &'static Value {
    static TEMPLATE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    TEMPLATE.get_or_init(|| {
        serde_json::from_str(include_str!("../../resources/codex-api-models.json"))
            .expect("bundled Codex model metadata")
    })
}

impl Configuration {
    pub fn for_route(route: &Route, model: Option<&str>) -> Self {
        if route.profile.is_none()
            || route.config["official"] == true
            || route
                .profile
                .as_ref()
                .is_some_and(crate::ccswitch::Profile::is_official)
        {
            return Self::default();
        }
        let window = model.and_then(|model| crate::model_context::for_model(&route.config, model));
        let percent = model
            .and_then(|model| crate::model_context::effective_percent(&route.config, model))
            .unwrap_or(95);
        let mut ids = crate::providers::model_ids(&route.config);
        ids.extend(
            route.config["modelMetadata"]
                .as_object()
                .into_iter()
                .flat_map(|metadata| metadata.keys().cloned()),
        );
        ids.truncate(200);
        if let Some(model) = model {
            ids.push(model.to_owned());
        }
        ids.sort();
        ids.dedup();
        let mapped = ids
            .into_iter()
            .filter(|id| !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control))
            .filter_map(|model| {
                crate::model_context::for_model(&route.config, &model).map(|window| ModelWindow {
                    percent: crate::model_context::effective_percent(&route.config, &model)
                        .unwrap_or(95),
                    model,
                    window,
                })
            })
            .collect();
        Self {
            managed: true,
            model: window.and(model.map(str::to_owned)),
            window,
            effective_percent: percent,
            mapped,
        }
    }

    fn catalog(&self) -> Option<Value> {
        if self.mapped.is_empty() {
            return None;
        }
        // Keep the SDK's bundled tool metadata and instructions. Each verified
        // API model gets its own limits; custom models retain the native fallback
        // instructions. Models without capacity metadata do not get a new row.
        let template = template();
        let mut rows = template["models"].as_array().unwrap().clone();
        // An API parent's global ceiling must not enlarge an unmapped native child.
        for row in &mut rows {
            row["max_context_window"] = row["context_window"].clone();
        }
        for mapped in &self.mapped {
            let mut selected = template["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["slug"] == mapped.model)
                .cloned()
                .unwrap_or_else(|| template["fallback"].clone());
            selected["slug"] = json!(mapped.model);
            selected["display_name"] = json!(mapped.model);
            selected["context_window"] = json!(mapped.window);
            selected["max_context_window"] = json!(mapped.window);
            selected["effective_context_window_percent"] = json!(mapped.percent);
            selected["auto_compact_token_limit"] =
                json!((mapped.window * mapped.percent / 100 * 9 / 10).max(1));
            rows.retain(|row| row["slug"] != mapped.model);
            rows.push(selected);
        }
        Some(json!({"models": rows}))
    }

    pub fn apply(&self, overrides: &mut Vec<String>, data_dir: &Path) -> Result<(), String> {
        if !self.managed {
            return Ok(());
        }
        // A provider's saved default may describe another selected model. A fresh
        // process must also drop the previous app-generated limits for unknown IDs.
        overrides.retain(|value| {
            !matches!(
                value.split('=').next().map(str::trim),
                Some(
                    "model_context_window"
                        | "model_auto_compact_token_limit"
                        | "model_catalog_json"
                )
            )
        });
        let Some(catalog) = self.catalog() else {
            return Ok(());
        };
        let ceiling = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| crate::model_context::positive(&row["context_window"]))
            .chain(crate::model_context::positive(
                &template()["fallback"]["context_window"],
            ))
            .max()
            .unwrap();
        let path = save_catalog(data_dir, &catalog)?;
        overrides.push(format!(
            "model_catalog_json={}",
            toml::Value::String(path.to_string_lossy().into_owned())
        ));
        // Each model caps this ceiling at its own max. Compaction remains per-row:
        // a global limit would overwrite a small child's effective-window reserve.
        overrides.push(format!("model_context_window={ceiling}"));
        Ok(())
    }
}

fn save_catalog(data_dir: &Path, catalog: &Value) -> Result<PathBuf, String> {
    let bytes = serde_json::to_vec(catalog).map_err(|error| error.to_string())?;
    let digest = Sha256::digest(&bytes);
    let directory = data_dir.join("codex-model-contexts");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("无法保存 Codex 模型目录：{error}"))?;
    let path = directory.join(format!("{digest:x}.json"));
    match std::fs::read(&path) {
        Ok(saved) => {
            if saved.starts_with(&[0xef, 0xbb, 0xbf]) || std::str::from_utf8(&saved).is_err() {
                return Err("Codex 模型目录不是 UTF-8 无 BOM，未修改其编码".into());
            }
            if saved != bytes {
                return Err("Codex 模型目录已损坏，未覆盖已有文件".into());
            }
            return Ok(path);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("无法读取 Codex 模型目录：{error}")),
    }
    let temporary = directory.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, &bytes)
        .map_err(|error| format!("无法保存 Codex 模型目录：{error}"))?;
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            return Err(format!("无法保存 Codex 模型目录：{error}"));
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn route(config: Value) -> Route {
        Route {
            id: "api".into(),
            profile: Some(crate::ccswitch::Profile {
                id: "api".into(),
                agent: "codex".into(),
                name: "API".into(),
                config: config.clone(),
            }),
            config,
        }
    }
    #[test]
    fn each_selected_api_model_has_its_own_real_catalog_window_and_compaction_limit() {
        let route = route(json!({"apiKey":"fixture","modelMetadata":{
            "large":{"contextWindow":1000000,"effectiveContextWindowPercent":80},
            "small":{"contextWindow":64000}
        }}));
        for (model, window, percent, threshold) in
            [("large", 1000000, 80, 720000), ("small", 64000, 95, 54720)]
        {
            let configuration = Configuration::for_route(&route, Some(model));
            let catalog = configuration.catalog().unwrap();
            let row = catalog["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["slug"] == model)
                .unwrap();
            assert_eq!(row["context_window"], window);
            // Codex clamps config overrides to this max; leaving the 272K fallback
            // here would silently defeat larger custom model context windows.
            assert_eq!(row["max_context_window"], window);
            assert_eq!(row["effective_context_window_percent"], percent);
            assert_eq!(row["auto_compact_token_limit"], threshold);
            assert!(
                row["model_messages"]["instructions_template"]
                    .as_str()
                    .unwrap()
                    .len()
                    > 1000
            );
            assert_eq!(row["shell_type"], "unified_exec");
            assert!(row["guardian"].is_null());
        }
        let unknown = Configuration::for_route(&route, Some("unknown"))
            .catalog()
            .unwrap();
        assert!(unknown["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["slug"] != "unknown"));
    }
    #[test]
    fn known_native_metadata_is_retained_for_api_models_and_never_applied_to_accounts_or_local_routes(
    ) {
        let api = route(json!({"apiKey":"fixture"}));
        let configuration = Configuration::for_route(&api, Some("gpt-6-astra"));
        let catalog = configuration.catalog().unwrap();
        let row = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["slug"] == "gpt-6-astra")
            .unwrap();
        let bundled: Value =
            serde_json::from_str(include_str!("../../resources/codex-api-models.json")).unwrap();
        let original = bundled["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["slug"] == "gpt-6-astra")
            .unwrap();
        assert_eq!(row["model_messages"], original["model_messages"]);
        assert_eq!(
            row["experimental_supported_tools"],
            original["experimental_supported_tools"]
        );
        let official = route(json!({"official":true}));
        let native = Route {
            id: "@local".into(),
            config: json!({"modelMetadata":{"gpt-6-astra":{"contextWindow":1050000}}}),
            profile: None,
        };
        for route in [official, native] {
            assert_eq!(
                Configuration::for_route(&route, Some("gpt-6-astra")),
                Configuration::default()
            );
        }
    }
    #[test]
    fn unknown_selection_removes_prior_provider_context_overrides() {
        let data_dir =
            std::env::temp_dir().join(format!("supercode-codex-context-{}", uuid::Uuid::new_v4()));
        let route = route(
            json!({"apiKey":"fixture","models":["small"],"config":"model='small'\nmodel_context_window=64000\nmodel_auto_compact_token_limit=57000"}),
        );
        let mut overrides = crate::ccswitch::codex_overrides(&route.config).unwrap();
        let unknown = Configuration::for_route(&route, Some("unknown"));
        unknown.apply(&mut overrides, &data_dir).unwrap();
        assert!(overrides
            .iter()
            .all(|value| value != "model_context_window=64000"
                && !value.starts_with("model_auto_compact_token_limit=")));
        let ceiling: u64 = overrides
            .iter()
            .find(|value| value.starts_with("model_context_window="))
            .unwrap()
            .split('=')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        assert!(ceiling >= 272000);
        let catalog = unknown.catalog().unwrap();
        assert!(catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["slug"] != "unknown"
                && row["context_window"].as_u64().unwrap() <= ceiling));
        assert!(overrides.iter().any(|value| value == "model=\"small\""));
        clean_fixture(&data_dir);
    }
    #[test]
    fn variants_keep_the_declared_effective_percentage() {
        let api = route(
            json!({"apiKey":"fixture","modelMetadata":{"k3":{"contextWindow":1048576,"effectiveContextWindowPercent":80}}}),
        );
        let configuration = Configuration::for_route(&api, Some("k3[1m]"));
        assert_eq!(configuration.effective_percent, 80);
        let catalog = configuration.catalog().unwrap();
        let row = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["slug"] == "k3[1m]")
            .unwrap();
        assert_eq!(row["auto_compact_token_limit"], 754974);
    }
    #[test]
    fn real_launch_catalog_is_utf8_immutable_and_reused_without_overwriting_corruption() {
        let data_dir =
            std::env::temp_dir().join(format!("supercode-codex-context-{}", uuid::Uuid::new_v4()));
        let api = route(
            json!({"apiKey":"fixture","modelMetadata":{"测试-model":{"contextWindow":1000000}}}),
        );
        let configuration = Configuration::for_route(&api, Some("测试-model"));
        let mut overrides = vec!["model_context_window=272000".into()];
        configuration.apply(&mut overrides, &data_dir).unwrap();
        let catalog_override = overrides
            .iter()
            .find(|value| value.starts_with("model_catalog_json="))
            .unwrap();
        let table: toml::Table = toml::from_str(catalog_override).unwrap();
        let path = PathBuf::from(table["model_catalog_json"].as_str().unwrap());
        let original = std::fs::read(&path).unwrap();
        assert!(!original.starts_with(&[0xef, 0xbb, 0xbf]));
        let native_catalog: Value = serde_json::from_slice(&original).unwrap();
        assert!(native_catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["slug"] == "测试-model" && row["max_context_window"] == 1000000));
        let mut repeated = vec![];
        configuration.apply(&mut repeated, &data_dir).unwrap();
        assert_eq!(repeated, overrides);
        std::fs::write(&path, [0xff]).unwrap();
        assert!(configuration
            .apply(&mut vec![], &data_dir)
            .unwrap_err()
            .contains("UTF-8"));
        assert_eq!(std::fs::read(&path).unwrap(), [0xff]);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(data_dir.join("codex-model-contexts")).unwrap();
        std::fs::remove_dir(data_dir).unwrap();
    }
    fn clean_fixture(data_dir: &Path) {
        let directory = data_dir.join("codex-model-contexts");
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            assert!(path.starts_with(&directory));
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(directory).unwrap();
        std::fs::remove_dir(data_dir).unwrap();
    }
    #[tokio::test]
    async fn controlled_cli_peer_reads_real_model_catalog_and_keeps_native_history_across_model_changes(
    ) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let data_dir = std::env::temp_dir().join(format!(
            "supercode-codex-context-peer-{}",
            uuid::Uuid::new_v4()
        ));
        let api = route(
            json!({"apiKey":"fixture","config":"model='large'\nmodel_context_window=272000\nmodel_auto_compact_token_limit=250000","models":["large","small","unknown"],"modelMetadata":{
                "large":{"contextWindow":1000000,"effectiveContextWindowPercent":80},"small":{"contextWindow":64000,"effectiveContextWindowPercent":75}
            }}),
        );
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex-context-peer.cjs");
        for model in ["large", "small", "unknown", "large"] {
            let policy = Configuration::for_route(&api, Some(model));
            let mut overrides = crate::ccswitch::codex_overrides(&api.config).unwrap();
            policy.apply(&mut overrides, &data_dir).unwrap();
            let expected=json!({"model":model,"native":"existing-history-thread","overrides":overrides,"models":{
                "large":{"window":1000000,"percent":80,"auto":720000},"small":{"window":64000,"percent":75,"auto":43200}
            }}).to_string();
            let fixture_path = fixture.to_string_lossy();
            let mut child =
                crate::process::command(Path::new("node"), &[fixture_path.as_ref(), &expected])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::inherit())
                    .spawn()
                    .unwrap();
            let mut input = child.stdin.take().unwrap();
            let mut output = BufReader::new(child.stdout.take().unwrap());
            let (method,params)=crate::commands::codex_thread_request(Some("existing-history-thread"),false,
                json!({"model":model,"cwd":"fixture","developerInstructions":"unchanged","serviceName":"supercode"})).unwrap();
            for (id, method, params) in [
                (1, "initialize", json!({})),
                (2, method, params),
                (
                    3,
                    "turn/start",
                    json!({"threadId":"existing-history-thread","model":model,"input":[]}),
                ),
            ] {
                let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
                    .to_string()
                    + "\n";
                input.write_all(request.as_bytes()).await.unwrap();
                input.flush().await.unwrap();
                let mut line = String::new();
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    output.read_line(&mut line),
                )
                .await
                .unwrap()
                .unwrap();
                let reply: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(reply["id"], id);
                assert!(reply.get("error").is_none());
                if id == 2 {
                    assert_eq!(reply["result"]["thread"]["id"], "existing-history-thread");
                }
            }
            drop(input);
            assert!(child.wait().await.unwrap().success());
        }
        assert!(crate::commands::codex_thread_request(
            Some("existing-history-thread"),
            true,
            json!({})
        )
        .is_none());
        clean_fixture(&data_dir);
    }
}
