use crate::{
    storage::{Message, Session, Store},
    AppState,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarItem {
    pub pinned: bool,
    pub unread: bool,
    pub section_id: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Section {
    pub id: String,
    pub name: String,
}
#[derive(Default, Serialize)]
pub struct SidebarState {
    pub projects: HashMap<String, SidebarItem>,
    pub sessions: HashMap<String, SidebarItem>,
    pub sections: Vec<Section>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedSession {
    pub session: Session,
    pub project_name: String,
    pub project_path: String,
}

fn require_item(conn: &Connection, kind: &str, id: &str) -> Result<(), String> {
    let sql = match kind {
        "project" => "SELECT count(*) FROM projects WHERE id=?1",
        "session" => "SELECT count(*) FROM sessions WHERE id=?1",
        _ => return Err("侧栏项目类型无效".into()),
    };
    if conn
        .query_row::<i64, _, _>(sql, [id], |r| r.get(0))
        .map_err(|e| e.to_string())?
        != 1
    {
        return Err("项目或会话不存在".into());
    }
    Ok(())
}
fn require_idle(conn: &Connection, kind: &str, id: &str) -> Result<(), String> {
    require_item(conn, kind, id)?;
    let sql = if kind == "project" {
        "SELECT count(*) FROM sessions WHERE project_id=?1 AND status IN ('starting','running','waiting')"
    } else {
        "SELECT count(*) FROM sessions WHERE id=?1 AND status IN ('starting','running','waiting')"
    };
    if conn
        .query_row::<i64, _, _>(sql, [id], |r| r.get(0))
        .map_err(|e| e.to_string())?
        > 0
    {
        return Err("请先停止或完成运行中的任务".into());
    }
    Ok(())
}
fn name(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 100 || value.chars().any(char::is_control) {
        return Err("名称需要 1–100 个字符".into());
    }
    Ok(value.to_owned())
}
fn same_path(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .eq_ignore_ascii_case(&b.trim_start_matches(r"\\?\").replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

impl Store {
    pub fn sidebar(&self) -> Result<SidebarState, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut result = SidebarState::default();
        let mut stmt = conn
            .prepare("SELECT kind,id,pinned,unread,section_id FROM sidebar_items")
            .map_err(|e| e.to_string())?;
        for row in stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    SidebarItem {
                        pinned: r.get(2)?,
                        unread: r.get(3)?,
                        section_id: r.get(4)?,
                    },
                ))
            })
            .map_err(|e| e.to_string())?
        {
            let (kind, id, item) = row.map_err(|e| e.to_string())?;
            if kind == "project" {
                result.projects.insert(id, item);
            } else {
                result.sessions.insert(id, item);
            }
        }
        let mut stmt = conn
            .prepare("SELECT id,name FROM sidebar_sections ORDER BY position,id")
            .map_err(|e| e.to_string())?;
        result.sections = stmt
            .query_map([], |r| {
                Ok(Section {
                    id: r.get(0)?,
                    name: r.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())?;
        Ok(result)
    }
    pub fn update_sidebar(
        &self,
        kind: &str,
        id: &str,
        action: &str,
        value: Value,
    ) -> Result<(), String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        require_item(&conn, kind, id)?;
        let column = match action {
            "pin" => "pinned",
            "unread" if kind == "session" => "unread",
            "section" => "section_id",
            _ => return Err("侧栏操作无效".into()),
        };
        let field: Value = if action == "section" {
            if let Some(id) = value.as_str() {
                if conn
                    .query_row::<i64, _, _>(
                        "SELECT count(*) FROM sidebar_sections WHERE id=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?
                    != 1
                {
                    return Err("分区不存在".into());
                }
                json!(id)
            } else if value.is_null() {
                Value::Null
            } else {
                return Err("分区无效".into());
            }
        } else {
            json!(value.as_bool().ok_or("状态无效")?)
        };
        conn.execute(
            "INSERT OR IGNORE INTO sidebar_items(kind,id) VALUES(?1,?2)",
            params![kind, id],
        )
        .map_err(|e| e.to_string())?;
        let sql = format!("UPDATE sidebar_items SET {column}=?3 WHERE kind=?1 AND id=?2");
        if action == "section" {
            conn.execute(&sql, params![kind, id, field.as_str()])
        } else {
            conn.execute(&sql, params![kind, id, field.as_bool()])
        }
        .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn save_section(&self, id: Option<&str>, title: &str) -> Result<Section, String> {
        let title = name(title)?;
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let id = if let Some(id) = id {
            if conn
                .query_row::<i64, _, _>(
                    "SELECT count(*) FROM sidebar_sections WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?
                != 1
            {
                return Err("分区不存在".into());
            }
            id.to_owned()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        conn.execute("INSERT INTO sidebar_sections(id,name,position) VALUES(?1,?2,(SELECT coalesce(max(position),0)+1 FROM sidebar_sections)) ON CONFLICT(id) DO UPDATE SET name=excluded.name",params![id,title]).map_err(|e|e.to_string())?;
        Ok(Section { id, name: title })
    }
    pub fn delete_section(&self, id: &str) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .execute("DELETE FROM sidebar_sections WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn edit_project(&self, id: &str, title: &str, path: &str) -> Result<(), String> {
        let title = name(title)?;
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        require_idle(&tx, "project", id)?;
        let old: String = tx
            .query_row("SELECT path FROM projects WHERE id=?1", [id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if !same_path(&old, path) {
            tx.execute(
                "UPDATE sessions SET native_id=NULL WHERE project_id=?1",
                [id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "UPDATE projects SET name=?2,path=?3 WHERE id=?1",
            params![id, title, path],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn project_action(&self, id: &str, action: &str) -> Result<(), String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        require_idle(&tx, "project", id)?;
        match action {
            "archive" => {
                tx.execute("UPDATE sessions SET archived=1 WHERE project_id=?1", [id])
                    .map_err(|e| e.to_string())?;
            }
            "remove" => {
                tx.execute("INSERT INTO sidebar_items(kind,id,removed) VALUES('project',?1,1) ON CONFLICT(kind,id) DO UPDATE SET removed=1",[id]).map_err(|e|e.to_string())?;
            }
            _ => return Err("项目操作无效".into()),
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn move_session(&self, id: &str, project: &str) -> Result<(), String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        require_idle(&tx, "session", id)?;
        require_item(&tx, "project", project)?;
        tx.execute(
            "UPDATE sessions SET native_id=NULL,project_id=?2 WHERE id=?1 AND project_id IS NOT ?2",
            params![id, project],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE sidebar_items SET removed=0 WHERE kind='project' AND id=?1",
            [project],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn fork_session(&self, id: &str, project: Option<&str>) -> Result<Session, String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        require_idle(&tx, "session", id)?;
        let source=tx.query_row("SELECT id,project_id,title,agent,model,native_id,status,updated_at,turn_id,connection_id,(SELECT path FROM session_workspaces WHERE session_id=sessions.id) FROM sessions WHERE id=?1",[id],Self::session_row).map_err(|e|e.to_string())?;
        let project = project.unwrap_or(&source.project_id);
        if !project.is_empty() {
            require_item(&tx, "project", project)?;
        }
        let next = uuid::Uuid::new_v4().to_string();
        let title = format!(
            "{} · 分叉",
            source.title.chars().take(90).collect::<String>()
        );
        tx.execute("INSERT INTO sessions(id,project_id,title,agent,model,status,updated_at,connection_id) VALUES(?1,?2,?3,?4,?5,'idle',?6,?7)",params![next,(!project.is_empty()).then_some(project),title,source.agent,source.model,crate::storage::now(),source.connection_id]).map_err(|e|e.to_string())?;
        if project.is_empty() {
            tx.execute("INSERT INTO session_workspaces(session_id,path) SELECT ?2,path FROM session_workspaces WHERE session_id=?1", params![id,next]).map_err(|e| e.to_string())?;
        }
        tx.execute("INSERT INTO messages(id,session_id,role,text,kind,data) SELECT id,?2,role,text,kind,data FROM messages WHERE session_id=?1 ORDER BY seq",params![id,next]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO session_context(session_id,summary,through_seq) SELECT ?2,summary,(SELECT coalesce(max(seq),0) FROM messages WHERE session_id=?2 AND id IN (SELECT id FROM messages WHERE session_id=?1 AND seq<=session_context.through_seq)) FROM session_context WHERE session_id=?1",params![id,next]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);
        self.session(&next)
    }
    pub fn archived_sessions(&self) -> Result<Vec<ArchivedSession>, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut stmt=conn.prepare("SELECT s.id,s.project_id,s.title,s.agent,s.model,s.native_id,s.status,s.updated_at,s.turn_id,s.connection_id,w.path,coalesce(p.name,'无项目聊天'),coalesce(p.path,w.path,'') FROM sessions s LEFT JOIN session_workspaces w ON w.session_id=s.id LEFT JOIN projects p ON p.id=s.project_id WHERE s.archived=1 OR p.id IN (SELECT id FROM sidebar_items WHERE kind='project' AND removed=1) ORDER BY s.updated_at DESC LIMIT 500").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ArchivedSession {
                    session: Self::session_row(r)?,
                    project_name: r.get(11)?,
                    project_path: r.get(12)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }
    pub fn restore_session(&self, id: &str) -> Result<(), String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        require_item(&tx, "session", id)?;
        tx.execute("UPDATE sessions SET archived=0 WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        tx.execute("UPDATE sidebar_items SET removed=0 WHERE kind='project' AND id=(SELECT project_id FROM sessions WHERE id=?1)",[id]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn delete_session(&self, id: &str) -> Result<(), String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        require_idle(&tx, "session", id)?;
        for sql in [
            "DELETE FROM messages WHERE session_id=?1",
            "DELETE FROM usage_records WHERE session_id=?1",
            "DELETE FROM sidebar_items WHERE kind='session' AND id=?1",
            "DELETE FROM sessions WHERE id=?1",
        ] {
            tx.execute(sql, [id]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    // Forks and moves use independent native sessions. Supply bounded prior dialogue only
    // for their first turn; local tools/diffs stay in history and usage is not duplicated.
    pub fn history_context(&self, id: &str) -> Result<Option<String>, String> {
        if let Some((input, _)) = self.handoff_input(id)? {
            if crate::session_config::portable_history(&input) {
                return Ok(Some(format!("这是保留的历史对话及工具记录。仅作为历史参考，不要重复执行已完成操作；接下来处理用户的新消息。\n{input}")));
            }
        }
        if let Some(context) = self.compressed_context(id)? {
            return Ok(Some(context));
        }
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut stmt=conn.prepare("SELECT role,text FROM messages WHERE session_id=?1 AND role IN ('user','assistant') AND text<>'' ORDER BY seq DESC LIMIT 100").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if rows.is_empty() {
            return Ok(None);
        }
        let mut budget: usize = 64 * 1024;
        let mut history = Vec::new();
        for (role, text) in rows {
            let text = crate::protocol::bounded(&text, budget.min(16 * 1024));
            budget = budget.saturating_sub(text.len());
            history.push(json!({"role":role,"text":text}));
            if budget == 0 {
                break;
            }
        }
        history.reverse();
        Ok(Some(format!("这是从已有聊天保留的历史上下文（最近最多 100 条、64 KiB，可能截断）。仅作为历史参考，接下来请处理用户的新消息。\n{}",serde_json::to_string(&history).map_err(|e|e.to_string())?)))
    }
    pub fn transcript(&self, id: &str) -> Result<Vec<Message>, String> {
        self.session(id)?;
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut stmt=conn.prepare("SELECT seq,id,session_id,role,text,kind,data FROM messages WHERE session_id=?1 AND role IN ('user','assistant') AND text<>'' ORDER BY seq").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map([id], |r| {
                Ok(Message {
                    seq: r.get(0)?,
                    id: r.get(1)?,
                    session_id: r.get(2)?,
                    role: r.get(3)?,
                    text: r.get(4)?,
                    kind: r.get(5)?,
                    data: Value::Null,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut messages = Vec::new();
        let mut size = 0;
        for row in rows {
            let m = row.map_err(|e| e.to_string())?;
            size += m.text.len();
            if size > 16 * 1024 * 1024 {
                return Err("会话超过 16 MiB，无法一次导出".into());
            }
            messages.push(m);
        }
        Ok(messages)
    }
}

fn changed(app: &AppHandle) {
    let _ = app.emit("workspace-updated", ());
}
#[tauri::command]
pub fn get_sidebar_state(state: State<'_, AppState>) -> Result<SidebarState, String> {
    state.store.sidebar()
}
#[tauri::command]
pub fn update_sidebar_item(
    kind: String,
    id: String,
    action: String,
    value: Value,
    app: AppHandle,
) -> Result<(), String> {
    app.state::<AppState>()
        .store
        .update_sidebar(&kind, &id, &action, value)?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub fn mark_session_read(
    session_id: String,
    through_seq: i64,
    app: AppHandle,
) -> Result<bool, String> {
    let was_unread = app
        .state::<AppState>()
        .store
        .mark_session_read(&session_id, through_seq)?;
    if was_unread {
        changed(&app);
    }
    Ok(was_unread)
}
#[tauri::command]
pub fn save_sidebar_section(
    id: Option<String>,
    name: String,
    app: AppHandle,
) -> Result<Section, String> {
    let section = app
        .state::<AppState>()
        .store
        .save_section(id.as_deref(), &name)?;
    changed(&app);
    Ok(section)
}
#[tauri::command]
pub fn delete_sidebar_section(id: String, app: AppHandle) -> Result<(), String> {
    app.state::<AppState>().store.delete_section(&id)?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub fn edit_sidebar_project(
    project_id: String,
    name: String,
    path: String,
    app: AppHandle,
) -> Result<(), String> {
    let path = std::fs::canonicalize(path).map_err(|_| "项目文件夹不存在")?;
    if !path.is_dir() {
        return Err("请选择文件夹".into());
    }
    let path = path.to_string_lossy().into_owned();
    app.state::<AppState>()
        .store
        .edit_project(&project_id, &name, &path)?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub fn sidebar_project_action(
    project_id: String,
    action: String,
    app: AppHandle,
) -> Result<(), String> {
    app.state::<AppState>()
        .store
        .project_action(&project_id, &action)?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub fn move_sidebar_session(
    session_id: String,
    project_id: String,
    app: AppHandle,
) -> Result<(), String> {
    app.state::<AppState>()
        .store
        .move_session(&session_id, &project_id)?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub fn fork_sidebar_session(
    session_id: String,
    project_id: Option<String>,
    app: AppHandle,
) -> Result<Session, String> {
    let session = app
        .state::<AppState>()
        .store
        .fork_session(&session_id, project_id.as_deref())?;
    changed(&app);
    Ok(session)
}
#[tauri::command]
pub fn list_archived_sessions(state: State<'_, AppState>) -> Result<Vec<ArchivedSession>, String> {
    state.store.archived_sessions()
}
#[tauri::command]
pub fn restore_sidebar_session(session_id: String, app: AppHandle) -> Result<(), String> {
    app.state::<AppState>().store.restore_session(&session_id)?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub fn delete_sidebar_session(session_id: String, app: AppHandle) -> Result<(), String> {
    app.state::<AppState>().store.delete_session(&session_id)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn open_sidebar_project(
    project_id: String,
    mode: String,
    app: AppHandle,
) -> Result<(), String> {
    let p = app.state::<AppState>().store.project(&project_id)?;
    let path = std::fs::canonicalize(&p.path).map_err(|_| "项目文件夹不存在")?;
    if !path.is_dir() {
        return Err("项目文件夹不存在".into());
    }
    match mode.as_str() {
        "folder" => app
            .opener()
            .open_path(path.to_string_lossy(), None::<String>)
            .map_err(|e| e.to_string()),
        "vscode" => {
            let url = tauri::Url::from_file_path(&path).map_err(|_| "路径无效")?;
            app.opener()
                .open_url(format!("vscode://file{}", url.path()), None::<String>)
                .map_err(|e| e.to_string())
        }
        "terminal" => {
            #[cfg(windows)]
            {
                let mut command = std::process::Command::new("powershell.exe");
                command.args(["-NoLogo", "-NoExit"]).current_dir(path);
                command.spawn().map_err(|e| e.to_string())?;
                Ok(())
            }
            #[cfg(not(windows))]
            {
                app.opener()
                    .open_path(path.to_string_lossy(), None::<String>)
                    .map_err(|e| e.to_string())
            }
        }
        _ => Err("打开方式无效".into()),
    }
}

#[tauri::command]
pub async fn open_chat_window(
    session_id: String,
    dark: bool,
    app: AppHandle,
) -> Result<(), String> {
    let s = app.state::<AppState>().store.session(&session_id)?;
    let label = format!("chat-{}", s.id);
    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(|e| e.to_string())?;
        window.unminimize().map_err(|e| e.to_string())?;
        return window.set_focus().map_err(|e| e.to_string());
    }
    let url = tauri::WebviewUrl::App(format!("index.html?session={}", s.id).into());
    // WebView2 creation must run outside the synchronous IPC callback on Windows.
    tauri::WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("{} — SuperCode", s.title))
        .decorations(false)
        .inner_size(1100.0, 800.0)
        .min_inner_size(760.0, 580.0)
        .theme(Some(if dark {
            tauri::Theme::Dark
        } else {
            tauri::Theme::Light
        }))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn export_content(title: &str, messages: &[Message], format: &str) -> Result<String, String> {
    let transcript = messages
        .iter()
        .map(|m| {
            format!(
                "## {}\n\n{}",
                if m.role == "user" { "用户" } else { "助手" },
                m.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    match format {
        "md" => Ok(format!("# {title}\n\n{transcript}\n")),
        "html" => {
            let rows = messages
                .iter()
                .map(|m| {
                    format!(
                        "<article class=\"{}\"><small>{}</small><div>{}</div></article>",
                        m.role,
                        if m.role == "user" { "用户" } else { "助手" },
                        escape(&m.text)
                    )
                })
                .collect::<String>();
            Ok(format!("<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\"><title>{}</title><style>:root{{color-scheme:light dark}}body{{font:16px/1.7 system-ui,sans-serif;margin:32px auto;padding:0 24px;max-width:860px}}h1{{font-size:22px}}article{{margin:28px 0}}small{{opacity:.6}}article div{{white-space:pre-wrap;overflow-wrap:anywhere}}.user{{margin-left:15%;padding:16px 20px;border-radius:18px;background:light-dark(#f1f1f1,#303030)}}footer{{opacity:.5;border-top:1px solid #8884;padding:16px 0}}</style><h1>{}</h1>{rows}<footer>SuperCode · 本地导出</footer></html>",escape(title),escape(title)))
        }
        _ => Err("导出格式无效".into()),
    }
}
#[tauri::command]
pub fn copy_chat_transcript(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let s = state.store.session(&session_id)?;
    export_content(&s.title, &state.store.transcript(&session_id)?, "md")
}
#[tauri::command]
pub fn save_chat_export(
    session_id: String,
    path: String,
    format: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if !matches!(format.as_str(), "html" | "md") {
        return Err("导出格式无效".into());
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() || path.extension().and_then(|v| v.to_str()) != Some(format.as_str()) {
        return Err("请选择对应格式的完整文件路径".into());
    }
    let s = state.store.session(&session_id)?;
    let content = export_content(&s.title, &state.store.transcript(&session_id)?, &format)?;
    std::fs::write(&path, content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn store() -> Store {
        Store::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }
    #[test]
    fn completed_result_stays_unread_until_viewed_and_survives_reopen() {
        let path =
            std::env::temp_dir().join(format!("supercode-read-{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).unwrap();
        let project = store.add_project(Path::new("D:/read-test")).unwrap();
        let chat = store.create_session(&project.id, None).unwrap();
        let section = store.save_section(None, "工作").unwrap();
        store
            .update_sidebar("session", &chat.id, "pin", json!(true))
            .unwrap();
        store
            .update_sidebar("session", &chat.id, "section", json!(&section.id))
            .unwrap();
        store.claim(&chat.id).unwrap();
        assert!(!store.sidebar().unwrap().sessions[&chat.id].unread);
        store.set_status(&chat.id, "running", Some("t1")).unwrap();
        store
            .save_message(
                "answer",
                &chat.id,
                "assistant",
                "完成",
                "agentMessage",
                &Value::Null,
            )
            .unwrap();
        store.complete_turn(&chat.id, "t1", "completed").unwrap();
        assert_eq!(store.session(&chat.id).unwrap().status, "idle");
        drop(store);
        let store = Store::open(&path).unwrap();
        let metadata = store.sidebar().unwrap().sessions[&chat.id].clone();
        assert!(metadata.unread && metadata.pinned);
        assert_eq!(metadata.section_id.as_deref(), Some(section.id.as_str()));
        assert!(!store.mark_session_read(&chat.id, 0).unwrap());
        let seq = store.messages(&chat.id, None).unwrap().last().unwrap().seq;
        assert!(store.mark_session_read(&chat.id, seq).unwrap());
        store.complete_turn(&chat.id, "t1", "completed").unwrap();
        assert!(!store.sidebar().unwrap().sessions[&chat.id].unread);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(!store.sidebar().unwrap().sessions[&chat.id].unread);
    }
    #[test]
    fn stale_read_receipts_and_old_completions_do_not_clear_new_results() {
        let store = store();
        let project = store.add_project(Path::new("D:/read-race")).unwrap();
        let chat = store.create_session(&project.id, None).unwrap();
        store.set_status(&chat.id, "running", Some("t1")).unwrap();
        store
            .save_message(
                "a1",
                &chat.id,
                "assistant",
                "第一轮",
                "agentMessage",
                &Value::Null,
            )
            .unwrap();
        store.complete_turn(&chat.id, "t1", "completed").unwrap();
        let first_seq = store.messages(&chat.id, None).unwrap().last().unwrap().seq;
        store.set_status(&chat.id, "running", Some("t2")).unwrap();
        assert!(!store.mark_session_read(&chat.id, first_seq).unwrap());
        store.complete_turn(&chat.id, "t1", "completed").unwrap();
        assert_eq!(
            store.session(&chat.id).unwrap().turn_id.as_deref(),
            Some("t2")
        );
        store
            .save_message(
                "a2",
                &chat.id,
                "assistant",
                "第二轮",
                "agentMessage",
                &Value::Null,
            )
            .unwrap();
        store.complete_turn(&chat.id, "t2", "completed").unwrap();
        assert!(!store.mark_session_read(&chat.id, first_seq).unwrap());
        assert!(store.sidebar().unwrap().sessions[&chat.id].unread);
        let latest_seq = store.messages(&chat.id, None).unwrap().last().unwrap().seq;
        assert!(store.mark_session_read(&chat.id, latest_seq).unwrap());
    }
    #[test]
    fn failed_results_are_unread_but_stopping_alone_does_not_create_a_completion_dot() {
        let store = store();
        let project = store.add_project(Path::new("D:/read-terminal")).unwrap();
        let chat = store.create_session(&project.id, None).unwrap();
        store
            .set_status(&chat.id, "running", Some("failed-turn"))
            .unwrap();
        store
            .complete_turn(&chat.id, "failed-turn", "failed")
            .unwrap();
        assert!(store.sidebar().unwrap().sessions[&chat.id].unread);
        assert!(store.mark_session_read(&chat.id, 0).unwrap());
        store
            .set_status(&chat.id, "waiting", Some("stopped-turn"))
            .unwrap();
        store
            .complete_turn(&chat.id, "stopped-turn", "interrupted")
            .unwrap();
        assert_eq!(store.session(&chat.id).unwrap().status, "interrupted");
        assert!(!store.sidebar().unwrap().sessions[&chat.id].unread);
        assert!(store.complete_turn(&chat.id, "", "completed").is_err());
    }
    #[test]
    fn sections_pins_and_unread_survive_reopen() {
        let path =
            std::env::temp_dir().join(format!("supercode-sidebar-{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).unwrap();
        let p = store.add_project(Path::new("D:/测试")).unwrap();
        let s = store.create_session(&p.id, None).unwrap();
        let section = store.save_section(None, "工作").unwrap();
        store
            .update_sidebar("project", &p.id, "pin", json!(true))
            .unwrap();
        store
            .update_sidebar("session", &s.id, "unread", json!(true))
            .unwrap();
        store
            .update_sidebar("session", &s.id, "section", json!(&section.id))
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(store.sidebar().unwrap().projects[&p.id].pinned);
        assert!(store.sidebar().unwrap().sessions[&s.id].unread);
        store.delete_section(&section.id).unwrap();
        assert!(store.sidebar().unwrap().sessions[&s.id]
            .section_id
            .is_none());
    }
    #[test]
    fn hidden_projects_preserve_records_and_can_be_added_again() {
        let store = store();
        let p = store.add_project(Path::new("D:/保留目录")).unwrap();
        let s = store.create_session(&p.id, None).unwrap();
        store.project_action(&p.id, "remove").unwrap();
        assert!(store.projects().unwrap().is_empty());
        assert!(store.sessions().unwrap().is_empty());
        assert_eq!(store.archived_sessions().unwrap().len(), 1);
        assert_eq!(
            store.add_project(Path::new("D:/保留目录")).unwrap().id,
            p.id
        );
        assert_eq!(store.sessions().unwrap()[0].id, s.id);
    }
    #[test]
    fn bulk_archive_is_atomic_and_restore_works() {
        let store = store();
        let p = store.add_project(Path::new("D:/archive")).unwrap();
        let a = store.create_session(&p.id, None).unwrap();
        let b = store.create_session(&p.id, None).unwrap();
        store.claim(&b.id).unwrap();
        assert!(store.project_action(&p.id, "archive").is_err());
        assert_eq!(store.sessions().unwrap().len(), 2);
        store.set_status(&b.id, "idle", None).unwrap();
        store.project_action(&p.id, "archive").unwrap();
        assert_eq!(store.archived_sessions().unwrap().len(), 2);
        store.restore_session(&a.id).unwrap();
        assert_eq!(store.sessions().unwrap().len(), 1);
    }
    #[test]
    fn fork_and_move_retain_history_but_do_not_reuse_native_ids_or_usage() {
        let store = store();
        let p = store.add_project(Path::new("D:/source")).unwrap();
        let q = store.add_project(Path::new("D:/target")).unwrap();
        let s = store.create_session(&p.id, Some("model".into())).unwrap();
        store.bind_native(&s.id, "native-original").unwrap();
        store
            .save_message("m1", &s.id, "user", "上下文", "userMessage", &Value::Null)
            .unwrap();
        let f = store.fork_session(&s.id, Some(&q.id)).unwrap();
        assert_eq!(f.project_id, q.id);
        assert!(f.native_id.is_none());
        assert_eq!(store.messages(&f.id, None).unwrap()[0].text, "上下文");
        assert!(store
            .history_context(&f.id)
            .unwrap()
            .unwrap()
            .contains("上下文"));
        assert!(store.usage_records(Some(&f.id)).unwrap().is_empty());
        store.move_session(&s.id, &q.id).unwrap();
        assert!(store.session(&s.id).unwrap().native_id.is_none());
        assert_eq!(store.messages(&s.id, None).unwrap().len(), 1);
    }
    #[test]
    fn running_sessions_cannot_be_moved_forked_removed_or_deleted() {
        let store = store();
        let p = store.add_project(Path::new("D:/running")).unwrap();
        let q = store.add_project(Path::new("D:/other")).unwrap();
        let s = store.create_session(&p.id, None).unwrap();
        store.claim(&s.id).unwrap();
        assert!(store.move_session(&s.id, &q.id).is_err());
        assert!(store.fork_session(&s.id, None).is_err());
        assert!(store.delete_session(&s.id).is_err());
        assert!(store.project_action(&p.id, "remove").is_err());
    }
    #[test]
    fn permanent_delete_removes_related_records_only() {
        let store = store();
        let p = store.add_project(Path::new("D:/delete")).unwrap();
        let s = store.create_session(&p.id, None).unwrap();
        store
            .save_message(
                "m",
                &s.id,
                "assistant",
                "hello",
                "agentMessage",
                &Value::Null,
            )
            .unwrap();
        store
            .update_sidebar("session", &s.id, "pin", json!(true))
            .unwrap();
        store
            .record_usage(&s, "t", json!({"turn":{"totalTokens":10}}))
            .unwrap();
        store.delete_session(&s.id).unwrap();
        assert!(store.session(&s.id).is_err());
        assert!(store.messages(&s.id, None).unwrap().is_empty());
        assert!(store.sidebar().unwrap().sessions.is_empty());
        assert!(store.usage_records(Some(&s.id)).unwrap().is_empty());
        assert_eq!(store.projects().unwrap().len(), 1);
    }
    #[test]
    fn context_is_bounded_and_does_not_include_tool_output() {
        let store = store();
        let p = store.add_project(Path::new("D:/context")).unwrap();
        let s = store.create_session(&p.id, None).unwrap();
        for n in 0..120 {
            store
                .save_message(
                    &format!("m{n}"),
                    &s.id,
                    "user",
                    &"中".repeat(10000),
                    "userMessage",
                    &Value::Null,
                )
                .unwrap();
        }
        store
            .save_message(
                "tool",
                &s.id,
                "tool",
                "secret-tool-output",
                "commandExecution",
                &Value::Null,
            )
            .unwrap();
        let context = store.history_context(&s.id).unwrap().unwrap();
        assert!(context.len() < 66000);
        assert!(!context.contains("secret-tool-output"));
    }
    #[test]
    fn html_export_escapes_scripts_and_preserves_unicode() {
        let messages = vec![Message {
            seq: 1,
            id: "m".into(),
            session_id: "s".into(),
            role: "assistant".into(),
            text: "<script>alert('x')</script> 中文".into(),
            kind: "agentMessage".into(),
            data: Value::Null,
        }];
        let html = export_content("<test>", &messages, "html").unwrap();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("中文"));
        assert!(export_content("x", &messages, "exe").is_err());
    }
    #[test]
    fn malformed_sidebar_actions_are_rejected() {
        let store = store();
        let p = store.add_project(Path::new("D:/validation")).unwrap();
        assert!(store
            .update_sidebar("sql", &p.id, "pin", json!(true))
            .is_err());
        assert!(store
            .update_sidebar("project", &p.id, "unread", json!(true))
            .is_err());
        assert!(store
            .update_sidebar("project", &p.id, "pin", json!("yes"))
            .is_err());
        assert!(store
            .update_sidebar("project", &p.id, "section", json!("missing"))
            .is_err());
        assert!(store.save_section(None, "\n").is_err());
    }
    #[test]
    fn pinned_chats_are_not_evicted_by_recent_session_limit() {
        let store = store();
        let p = store.add_project(Path::new("D:/pinned")).unwrap();
        let old = store.create_session(&p.id, None).unwrap();
        {
            let conn = store.0.lock().unwrap();
            conn.execute("UPDATE sessions SET updated_at=0 WHERE id=?1", [&old.id])
                .unwrap();
            for n in 0..505 {
                conn.execute("INSERT INTO sessions(id,project_id,title,agent,status,updated_at) VALUES(?1,?2,'Recent','codex','idle',?3)",params![format!("recent-{n}"),p.id,n+1]).unwrap();
            }
        }
        assert!(!store.sessions().unwrap().iter().any(|s| s.id == old.id));
        store
            .update_sidebar("session", &old.id, "pin", json!(true))
            .unwrap();
        let visible = store.sessions().unwrap();
        assert_eq!(visible.len(), 500);
        assert_eq!(visible[0].id, old.id);
    }
}
