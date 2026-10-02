//! Commands from another device, into the app's own command handlers.
//!
//! Tauri's IPC entry (`Webview::on_message` with the app's invoke key)
//! takes the request as if the main window sent it, so every command works
//! remotely without a second implementation. That entry is not a stable
//! Tauri API: the Tauri version is pinned, and the tests below go through it.
//!
//! A command added to the app is closed to remote callers until it is listed
//! here. Every paired device gets the whole list, a phone included: it comes
//! in over this machine's own tailnet with a pairing token, and there it does
//! what the app's window does.

use serde_json::Value;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponse, InvokeResponseBody};
use tauri::webview::InvokeRequest;
use tauri::{AppHandle, Manager, Runtime};

/// The conversation: tracks, runs, decisions, workers, designs, knowledge.
///
/// Left out of both lists on purpose, for every device: anything that acts on
/// the PC's own screen (pickers, save dialogs, the file manager, opening files
/// or links there, agent sign-in in its browser, the app's own installer), and
/// who may reach this machine at all (pairing, the server and phone switches,
/// remote instances, GitHub sign-in).
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
    // routines
    "list_routines",
    "create_routine",
    "update_routine",
    "delete_routine",
    "routine_runs",
    "run_routine",
    "routine_cancel",
    "running_routines",
    // the app
    "agent_statuses",
    "app_info",
    "system_metrics",
    "get_setting",
    "set_setting",
    "ui_log",
    "last_crash",
    "log_level",
    "log_level_set",
    "diagnostics_report",
];

/// The machine: its shell, its folders, making and deleting things.
///
/// Once only a [`super::auth::Scope::Full`] device's (the owner's, over SSH or
/// GitHub); a phone, paired as `Conversation`, got [`ALLOWED`] alone and could
/// not so much as make a track. A phone is on this machine's own tailnet,
/// paired from its window, so it gets this too.
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
/// remote-instance switches. The embedding's address and model are open; its
/// key is a secret.
fn setting_closed(key: &str) -> bool {
    key.starts_with("remote") || key.starts_with("github") || key.contains("key") || key.contains("token") || key.contains("secret")
}

/// Whether another device may make this call.
pub fn allowed(cmd: &str, args: &Value) -> Result<(), String> {
    if !(ALLOWED.contains(&cmd) || OWNER_ALLOWED.contains(&cmd)) {
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
        let call = |cmd: &str| allowed(cmd, &json!({}));
        assert!(call("list_tracks").is_ok());
        assert!(call("pick_folder").is_err());
        assert!(call("term_open").is_ok(), "the machine's shell");
        assert!(call("workspace_reveal").is_err());
        assert!(call("delete_track").is_ok());
        assert!(call("remote_server_set").is_err());
        assert!(call("github_login_start").is_err());
        assert!(allowed("get_setting", &json!({ "key": "theme" })).is_ok());
        assert!(allowed("get_setting", &json!({ "key": "github.client_id" })).is_err());
        assert!(allowed("get_setting", &json!({ "key": "knowledge.embed.key" })).is_err());
        assert!(allowed("set_setting", &json!({ "key": "remote.enabled" })).is_err());
    }

    /// A phone does what the app's window does: what the conversation needs,
    /// and the machine's shell, folders, tracks and routines. Before, a phone
    /// was refused every one of these but the first three.
    #[test]
    fn a_phone_does_what_the_window_does() {
        let call = |cmd: &str| allowed(cmd, &json!({}));
        for cmd in [
            "list_tracks",
            "conductor_prompt",
            "answer_decision",
            "create_track",
            "update_track",
            "delete_track",
            "browse_dirs",
            "make_dir",
            "term_open",
            "term_write",
            "term_close",
            "workspace_write",
            "design_apply",
            "download_agent",
            "knowledge_add",
            "knowledge_embed_test",
            "list_routines",
            "create_routine",
            "run_routine",
            "delete_routine",
            "ui_log",
            "last_crash",
            "log_level_set",
            "diagnostics_report",
        ] {
            assert!(call(cmd).is_ok(), "{cmd}");
        }
        // The embedding's settings, but not its key.
        for key in [
            "knowledge.embed.enabled",
            "knowledge.embed.url",
            "knowledge.embed.model",
            "knowledge.embed.dims",
            "knowledge.embed.rate",
        ] {
            assert!(allowed("get_setting", &json!({ "key": key })).is_ok(), "{key}");
            assert!(allowed("set_setting", &json!({ "key": key })).is_ok(), "{key}");
        }
    }

    /// Still closed to every device: the PC's own screen, and who may reach
    /// this machine. The closed settings stay closed too.
    #[test]
    fn the_pcs_screen_and_its_door_stay_closed() {
        for cmd in [
            "pick_folder",
            "pick_files",
            "workspace_save_as",
            "workspace_reveal",
            "logs_open",
            "design_open_file",
            "open_url",
            "login_agent",
            "update_download",
            "update_open",
            "phone_set",
            "phone_pair_link",
            "remote_server_set",
            "remote_server_drop_all",
            "remote_host_save",
            "instance_invoke",
            "show_instance",
            "github_login_start",
        ] {
            assert_eq!(allowed(cmd, &json!({})).unwrap_err(), format!("{cmd} is not available from another device"), "{cmd}");
        }
        for key in ["github.client_id", "knowledge.embed.key", "remote.enabled", "some.token", "a.secret"] {
            assert!(allowed("get_setting", &json!({ "key": key })).is_err(), "{key}");
            assert!(allowed("set_setting", &json!({ "key": key })).is_err(), "{key}");
        }
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
