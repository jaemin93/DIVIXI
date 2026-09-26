//! Commands from another device, into the app's own command handlers.
//!
//! Tauri's IPC entry (`Webview::on_message` with the app's invoke key)
//! takes the request as if the main window sent it, so every command works
//! remotely without a second implementation. That entry is not a stable
//! Tauri API: the Tauri version is pinned, and the tests below go through it.
//!
//! Only what the phone view needs is let through ([`ALLOWED`]); a command
//! added to the app is closed to remote callers until it is listed here.

use serde_json::Value;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponse, InvokeResponseBody};
use tauri::webview::InvokeRequest;
use tauri::{AppHandle, Manager, Runtime};

/// Commands another device may call. Left out on purpose: anything that
/// opens a window on the PC (pickers, save dialogs, the file manager,
/// opening files or links there), the terminal, agent sign-in and
/// download, deleting tracks, designs or knowledge, editing files, boards
/// and knowledge sources, and the remote-access controls themselves.
pub const ALLOWED: &[&str] = &[
    // tracks and conversations
    "list_tracks",
    "list_runs",
    "get_run",
    "run_events",
    "search_runs",
    "conductor_prompt",
    "conductor_state",
    "conductor_states",
    "conductor_open",
    "conductor_close",
    "conductor_cancel",
    // decisions and permissions
    "list_decisions",
    "answer_decision",
    "dismiss_decision",
    // worker reports and changes
    "worker_report",
    "worker_changes",
    "worker_file_diff",
    "worker_merge",
    "worker_discard",
    // the track's folder, read only
    "workspace_tree",
    "workspace_read",
    "workspace_search",
    "workspace_git_status",
    "workspace_git_diff",
    "file_stats",
    "save_attachment",
    // designs, read only, and their chat
    "list_artifacts",
    "artifact_state",
    "artifact_prompt",
    "artifact_cancel",
    "design_doc",
    "design_extracts",
    // knowledge, read only
    "knowledge_sources",
    "knowledge_source_for",
    "knowledge_items",
    "knowledge_graph",
    "knowledge_entity_items",
    "knowledge_stats",
    "knowledge_overlaps",
    "knowledge_formats",
    "knowledge_context",
    "knowledge_embedding_status",
    // the app
    "agent_statuses",
    "app_info",
    "system_metrics",
    "get_setting",
    "set_setting",
];

/// Settings another device may not read or write: secrets and the remote
/// access switches.
fn setting_closed(key: &str) -> bool {
    key.starts_with("remote") || key.starts_with("knowledge.embed") || key.contains("key") || key.contains("token") || key.contains("secret")
}

/// Whether a remote caller may make this call.
pub fn allowed(cmd: &str, args: &Value) -> Result<(), String> {
    if !ALLOWED.contains(&cmd) {
        return Err(format!("{cmd} is not available from another device"));
    }
    if cmd == "get_setting" || cmd == "set_setting" {
        let key = args.get("key").and_then(|k| k.as_str()).unwrap_or_default();
        if setting_closed(key) {
            return Err(format!("the setting {key} is not available from another device"));
        }
    }
    Ok(())
}

/// Run a command as the main window would, and take its answer.
/// `Ok` carries what the command returned, `Err` what it rejected with.
pub async fn invoke<R: Runtime>(app: &AppHandle<R>, cmd: &str, args: Value) -> Result<Value, Value> {
    let window = app.get_webview_window("main").ok_or_else(|| Value::String("the app has no window".into()))?;
    let webview = window.as_ref().clone();
    let url = webview.url().map_err(|e| Value::String(e.to_string()))?;
    let request = InvokeRequest {
        cmd: cmd.to_string(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url,
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: app.invoke_key().to_string(),
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    // A command that is not async runs right where it is called: off the async workers.
    tauri::async_runtime::spawn_blocking(move || {
        webview.on_message(
            request,
            Box::new(move |_webview, _cmd, response, _ok, _err| {
                let _ = tx.send(response);
            }),
        );
    });
    match rx.await.map_err(|_| Value::String("the command gave no answer".into()))? {
        InvokeResponse::Ok(InvokeResponseBody::Json(json)) => Ok(serde_json::from_str(&json).unwrap_or(Value::Null)),
        InvokeResponse::Ok(InvokeResponseBody::Raw(bytes)) => {
            use base64::Engine;
            Ok(Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)))
        }
        InvokeResponse::Err(err) => Err(err.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tauri::command]
    fn add(a: i64, b: i64) -> i64 {
        a + b
    }

    #[tauri::command]
    async fn fails() -> Result<(), String> {
        Err("no".into())
    }

    #[test]
    fn remote_calls_go_through_the_apps_own_commands() {
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![add, fails])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let _window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let handle = app.handle().clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(rt.block_on(invoke(&handle, "add", json!({ "a": 2, "b": 3 }))), Ok(json!(5)));
        assert_eq!(rt.block_on(invoke(&handle, "fails", json!({}))), Err(json!("no")));
        assert!(rt.block_on(invoke(&handle, "missing", json!({}))).is_err());
    }

    #[test]
    fn only_listed_commands_and_open_settings() {
        assert!(allowed("list_tracks", &json!({})).is_ok());
        assert!(allowed("pick_folder", &json!({})).is_err());
        assert!(allowed("term_open", &json!({})).is_err());
        assert!(allowed("delete_track", &json!({})).is_err());
        assert!(allowed("get_setting", &json!({ "key": "theme" })).is_ok());
        assert!(allowed("get_setting", &json!({ "key": "knowledge.embed.key" })).is_err());
        assert!(allowed("set_setting", &json!({ "key": "remote.enabled" })).is_err());
    }
}
