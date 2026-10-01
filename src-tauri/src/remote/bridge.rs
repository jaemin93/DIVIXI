//! Commands from another device, into the app's own command handlers.
//!
//! Tauri's IPC entry (`Webview::on_message` with the app's invoke key)
//! takes the request as if the main window sent it, so every command works
//! remotely without a second implementation. That entry is not a stable
//! Tauri API: the Tauri version is pinned, and the tests below go through it.
//!
//! A command added to the app is closed to remote callers until it is listed
//! here, and how much of the list a caller gets depends on the scope its device
//! was admitted with ([`super::auth::Scope`]).

use serde_json::Value;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponse, InvokeResponseBody};
use tauri::webview::InvokeRequest;
use tauri::{AppHandle, Manager, Runtime};

use super::auth::Scope;

/// The conversation: what every remote caller may do, whatever its scope.
///
/// Left out on purpose: anything that opens a window on the PC (pickers, save
/// dialogs, the file manager, opening files or links there), the terminal,
/// agent sign-in and download, deleting tracks, designs or knowledge, editing
/// files, boards and knowledge sources, and the remote-access controls
/// themselves. Those are [`OWNER_ALLOWED`], and a [`Scope::Conversation`]
/// device does not get them.
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
    "worker_cancel",
    "worker_close",
    "worker_sessions",
    "workers_tidy",
    "parked_deliveries",
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

/// The machine: what a [`Scope::Full`] device may do on top of [`ALLOWED`].
///
/// Full scope says whoever came in is this Divixi's owner -- they minted a
/// pairing token on this machine over SSH, or signed in as the GitHub account
/// that owns it -- so they do here what its own window does, its terminal
/// included. Still closed even then: anything that opens a window or a browser
/// on the machine (pickers, save dialogs, the file manager, agent sign-in), and
/// the remote-instance and GitHub controls.
const OWNER_ALLOWED: &[&str] = &[
    "create_track",
    "update_track",
    "delete_track",
    "worker_delete",
    "detect_agents",
    "download_agent",
    "workspace_write",
    "create_artifact",
    "update_artifact",
    "delete_artifact",
    "export_artifact",
    "design_apply",
    "design_review",
    "design_undo",
    "design_add_blob",
    // Files the Divixi app brought over first (instance_upload).
    "design_add_files",
    // A shell on this machine, drawn on the app's PC (as VS Code Remote).
    "term_open",
    "term_write",
    "term_resize",
    "term_close",
    // This machine's folders, for picking a track's (and making one).
    "browse_dirs",
    "make_dir",
    "design_extract_retry",
    "knowledge_add",
    "knowledge_sync",
    "knowledge_embed_test",
    "knowledge_embed_now",
    "knowledge_default_config",
];

/// Settings another device may not read or write: secrets, GitHub and the
/// remote-instance switches.
fn setting_closed(key: &str) -> bool {
    key.starts_with("remote") || key.starts_with("github") || key.starts_with("knowledge.embed") || key.contains("key") || key.contains("token") || key.contains("secret")
}

/// Whether a device admitted with `scope` may make this call.
///
/// Until the scope existed this took the union unconditionally, so [`ALLOWED`]
/// never actually bounded anything and every remote caller -- including one
/// paired to the desktop app -- could open a shell. The two lists now mean what
/// their names say.
pub fn allowed(cmd: &str, args: &Value, scope: Scope) -> Result<(), String> {
    let in_scope = ALLOWED.contains(&cmd) || (scope == Scope::Full && OWNER_ALLOWED.contains(&cmd));
    if !in_scope {
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
    // The main window's own webview (Local): with remote instances shown in
    // the same window, it is no longer a "webview window" to Tauri.
    let webview = app.get_webview("main").ok_or_else(|| Value::String("the app has no window".into()))?;
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
        let full = |cmd: &str| allowed(cmd, &json!({}), Scope::Full);
        assert!(full("list_tracks").is_ok());
        assert!(full("pick_folder").is_err());
        assert!(full("term_open").is_ok(), "the owner's shell");
        assert!(full("workspace_reveal").is_err());
        assert!(full("delete_track").is_ok(), "the owner may");
        assert!(full("remote_server_set").is_err());
        assert!(full("github_login_start").is_err());
        assert!(allowed("get_setting", &json!({ "key": "theme" }), Scope::Full).is_ok());
        assert!(allowed("get_setting", &json!({ "key": "github.client_id" }), Scope::Full).is_err());
        assert!(allowed("get_setting", &json!({ "key": "knowledge.embed.key" }), Scope::Full).is_err());
        assert!(allowed("set_setting", &json!({ "key": "remote.enabled" }), Scope::Full).is_err());
    }

    /// The boundary between the two lists, from both sides. Before the scope
    /// existed `allowed` took their union, so every one of the `Conversation`
    /// refusals below was an `Ok` -- a device paired to the desktop app could
    /// open a shell on it.
    #[test]
    fn the_conversation_scope_is_the_conversation_only() {
        let chat = |cmd: &str| allowed(cmd, &json!({}), Scope::Conversation);
        let owner = |cmd: &str| allowed(cmd, &json!({}), Scope::Full);

        // ALLOWED: both scopes, and the things a phone is actually for.
        for cmd in ["list_tracks", "get_run", "conductor_prompt", "list_decisions", "answer_decision", "worker_report", "workspace_read"] {
            assert!(chat(cmd).is_ok(), "{cmd} is the conversation");
            assert!(owner(cmd).is_ok(), "{cmd} is the conversation");
        }

        // OWNER_ALLOWED: full only. The machine, not the conversation.
        for cmd in ["term_open", "term_write", "term_close", "delete_track", "workspace_write", "browse_dirs", "make_dir", "design_apply", "download_agent", "knowledge_add"] {
            assert_eq!(
                chat(cmd).unwrap_err(),
                format!("{cmd} is not available from another device"),
                "{cmd} is the machine, not the conversation",
            );
            assert!(owner(cmd).is_ok(), "{cmd} is still the owner's");
        }

        // On neither list: refused whatever the scope.
        for cmd in ["pick_files", "remote_server_set", "remote_pair_link", "github_login_start", "workspace_reveal"] {
            assert!(chat(cmd).is_err(), "{cmd}");
            assert!(owner(cmd).is_err(), "{cmd}");
        }

        // The closed settings are closed at every scope -- the scope narrows
        // what may be called, it never opens a second door into a setting.
        for key in ["github.client_id", "knowledge.embed.key", "remote.enabled"] {
            assert!(allowed("get_setting", &json!({ "key": key }), Scope::Full).is_err(), "{key}");
            assert!(allowed("get_setting", &json!({ "key": key }), Scope::Conversation).is_err(), "{key}");
        }
        assert!(allowed("set_setting", &json!({ "key": "theme" }), Scope::Conversation).is_ok());
    }

    /// Neither list may grow a command the other already has: the union is no
    /// longer taken, so an entry in both would make the scope meaningless for
    /// it without anything failing.
    #[test]
    fn the_two_lists_do_not_overlap() {
        let both: Vec<&str> = ALLOWED.iter().copied().filter(|c| OWNER_ALLOWED.contains(c)).collect();
        assert!(both.is_empty(), "in both lists: {both:?}");
    }
}
