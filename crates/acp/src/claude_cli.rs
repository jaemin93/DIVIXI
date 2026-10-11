//! Which Claude Code CLI the adapter runs.
//!
//! The adapter drives Claude Code through the agent SDK, and the SDK ships a
//! CLI of its own. Left alone, the adapter runs that one: a build pinned with
//! the adapter, so a session sees only the models that build knows about, not
//! the ones the user's own (newer) Claude Code offers. `CLAUDE_CODE_EXECUTABLE`
//! points the adapter at another CLI; Divixi sets it to the installed one when
//! that one runs and is at least as new as the bundled one.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::AgentSpec;

/// The variable the adapter reads for the CLI to run.
pub const CLAUDE_CLI_ENV: &str = "CLAUDE_CODE_EXECUTABLE";

/// The CLI the pinned adapter's SDK ships (`claudeCodeVersion` in
/// `@anthropic-ai/claude-agent-sdk`'s package.json). Used when that file
/// cannot be read, as under `npx`.
pub const CLAUDE_BUNDLED_CLI_VERSION: &str = "2.1.280";

/// Which CLI a Claude session runs, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeCli {
    /// The installed CLI.
    Installed { path: PathBuf, version: String },
    /// The one bundled with the adapter.
    Bundled { reason: String },
}

impl ClaudeCli {
    /// Decide from what the search found: the installed CLI if it answered
    /// `--version` with a version no older than `bundled`.
    pub fn choose(found: Option<&Path>, version_line: Option<&str>, bundled: &str) -> Self {
        let Some(path) = found else {
            return Self::Bundled {
                reason: "no installed claude CLI found".to_string(),
            };
        };
        let Some(line) = version_line else {
            return Self::Bundled {
                reason: format!("{} did not answer --version", path.display()),
            };
        };
        let Some(version) = parse_version(line) else {
            return Self::Bundled {
                reason: format!("{} reported no version: {line}", path.display()),
            };
        };
        match parse_version(bundled) {
            Some(min) if version < min => Self::Bundled {
                reason: format!("{} is {} and older than the bundled {bundled}", path.display(), show(version)),
            },
            _ => Self::Installed {
                path: path.to_path_buf(),
                version: show(version),
            },
        }
    }

    /// One log line saying which CLI Claude sessions run.
    pub fn log(&self) {
        match self {
            Self::Installed { path, version } => {
                tracing::info!(cli = %path.display(), version, "claude: using the installed Claude Code CLI")
            }
            Self::Bundled { reason } => tracing::info!(reason, "claude: using the adapter's bundled Claude Code CLI"),
        }
    }
}

impl AgentSpec {
    /// Point the Claude adapter at `cli`: sets or clears
    /// [`CLAUDE_CLI_ENV`] in the spec's environment.
    pub fn with_claude_cli(mut self, cli: &ClaudeCli) -> Self {
        self.env.retain(|(k, _)| k != CLAUDE_CLI_ENV);
        if let ClaudeCli::Installed { path, .. } = cli {
            self.env.push((CLAUDE_CLI_ENV.to_string(), path.to_string_lossy().into_owned()));
        }
        self
    }
}

/// Whether a spec's variable goes to the process. A spec is kept with the
/// detection result, so the CLI it names may have been uninstalled since:
/// then the variable is left out and the adapter runs its bundled CLI,
/// rather than failing to start one that is gone.
pub(crate) fn keep_env(name: &str, value: &str) -> bool {
    if name != CLAUDE_CLI_ENV || Path::new(value).is_file() {
        return true;
    }
    tracing::warn!(cli = value, "claude: the installed CLI is gone; using the adapter's bundled Claude Code CLI");
    false
}

/// `major.minor.patch` out of a `--version` line like `2.1.296 (Claude Code)`.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.')).find_map(|word| {
        let mut parts = word.split('.');
        let v = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
        Some(v)
    })
}

fn show((a, b, c): (u64, u64, u64)) -> String {
    format!("{a}.{b}.{c}")
}

/// The CLI version the adapter at `script` bundles, read from the SDK
/// installed beside it; [`CLAUDE_BUNDLED_CLI_VERSION`] when there is none.
pub fn bundled_cli_version(script: Option<&Path>) -> String {
    script
        .and_then(|s| {
            s.ancestors().find_map(|dir| {
                let manifest = dir.join("node_modules").join("@anthropic-ai").join("claude-agent-sdk").join("package.json");
                let text = std::fs::read_to_string(manifest).ok()?;
                let json: serde_json::Value = serde_json::from_str(&text).ok()?;
                json.get("claudeCodeVersion")?.as_str().map(str::to_owned)
            })
        })
        .unwrap_or_else(|| CLAUDE_BUNDLED_CLI_VERSION.to_string())
}

/// The file names a runnable `claude` goes by. On Windows only the native
/// `.exe`: the adapter spawns the path directly, which a `.cmd` shim does not
/// survive.
fn cli_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["claude.exe"]
    } else {
        &["claude"]
    }
}

/// Where Claude Code's installers put `claude`, beyond the PATH.
pub fn claude_cli_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(h) = home {
        // The native installer, every platform.
        dirs.push(h.join(".local").join("bin"));
        // The old per-user ("migrate-installer") install.
        dirs.push(h.join(".claude").join("local"));
        if !cfg!(windows) {
            dirs.push(h.join(".npm-global").join("bin"));
            dirs.push(h.join(".bun").join("bin"));
        }
    }
    if !cfg!(windows) {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
    }
    dirs
}

/// The first runnable `claude` in `dirs`, in order.
pub fn find_claude_cli_in(dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    dirs.into_iter().flat_map(|dir| cli_names().iter().map(move |n| dir.join(n))).find(|p| p.is_file())
}

/// The process PATH, then `extra` (the registry PATH, say), then the known
/// install folders.
pub fn claude_cli_search_dirs(extra: Vec<PathBuf>) -> Vec<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(extra);
    dirs.extend(claude_cli_dirs(home.as_deref()));
    dirs
}

/// First line of `cli --version`, waiting at most `timeout`. Blocking: for
/// the rare launch that has no detection result to go on.
pub fn cli_version_blocking(cli: &Path, timeout: Duration) -> Option<String> {
    let mut child = std::process::Command::new(cli)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let out = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_out_of_a_version_line() {
        assert_eq!(parse_version("2.1.296 (Claude Code)"), Some((2, 1, 296)));
        assert_eq!(parse_version("claude 2.10.3"), Some((2, 10, 3)));
        assert_eq!(parse_version("2.1.280"), Some((2, 1, 280)));
        assert_eq!(parse_version("2.1"), None);
        assert_eq!(parse_version("Claude Code"), None);
    }

    #[test]
    fn versions_compare_as_numbers_not_text() {
        let p = Path::new("/bin/claude");
        let pick = |v: &str| ClaudeCli::choose(Some(p), Some(v), "2.1.280");
        assert!(matches!(pick("2.1.296 (Claude Code)"), ClaudeCli::Installed { version, .. } if version == "2.1.296"));
        assert!(matches!(pick("2.1.280 (Claude Code)"), ClaudeCli::Installed { .. }), "the same build is fine");
        assert!(matches!(pick("2.2.0"), ClaudeCli::Installed { .. }));
        assert!(matches!(pick("2.1.1000"), ClaudeCli::Installed { .. }), "1000 > 280, though not as text");
        assert!(matches!(pick("2.1.279 (Claude Code)"), ClaudeCli::Bundled { reason } if reason.contains("older")));
        assert!(matches!(pick("1.9.999"), ClaudeCli::Bundled { .. }));
    }

    #[test]
    fn falls_back_to_the_bundled_cli() {
        assert!(matches!(ClaudeCli::choose(None, None, "2.1.280"), ClaudeCli::Bundled { reason } if reason.contains("no installed")));
        let p = Path::new("/bin/claude");
        assert!(matches!(ClaudeCli::choose(Some(p), None, "2.1.280"), ClaudeCli::Bundled { reason } if reason.contains("--version")));
        assert!(matches!(ClaudeCli::choose(Some(p), Some("oops"), "2.1.280"), ClaudeCli::Bundled { .. }));
    }

    #[test]
    fn the_spec_carries_the_installed_cli_only() {
        let installed = ClaudeCli::Installed {
            path: PathBuf::from("/x/claude"),
            version: "2.1.296".into(),
        };
        let spec = AgentSpec::node_script("adapter.js").with_claude_cli(&installed);
        assert_eq!(spec.env, [(CLAUDE_CLI_ENV.to_string(), "/x/claude".to_string())]);
        // Choosing again replaces it; falling back clears it.
        let spec = spec.with_claude_cli(&installed);
        assert_eq!(spec.env.len(), 1);
        let spec = spec.with_claude_cli(&ClaudeCli::Bundled { reason: "r".into() });
        assert!(spec.env.is_empty());
    }

    #[test]
    fn a_cli_that_is_gone_is_not_handed_to_the_adapter() {
        let root = scratch("gone");
        let cli = root.join(cli_names()[0]);
        std::fs::write(&cli, b"x").unwrap();
        assert!(keep_env(CLAUDE_CLI_ENV, &cli.to_string_lossy()));
        std::fs::remove_file(&cli).unwrap();
        assert!(!keep_env(CLAUDE_CLI_ENV, &cli.to_string_lossy()));
        assert!(keep_env("ANTHROPIC_API_KEY", "not a path"), "other variables pass as they are");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("divixi-claude-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_the_first_runnable_claude_in_order() {
        let root = scratch("find");
        let (a, b, c) = (root.join("a"), root.join("b"), root.join("c"));
        for d in [&a, &b, &c] {
            std::fs::create_dir_all(d).unwrap();
        }
        let name = cli_names()[0];
        std::fs::write(b.join(name), b"x").unwrap();
        std::fs::write(c.join(name), b"x").unwrap();
        // A shim that the adapter could not spawn is passed over.
        std::fs::write(a.join("claude.cmd"), b"x").unwrap();
        assert_eq!(find_claude_cli_in([a.clone(), b.clone(), c.clone()]), Some(b.join(name)));
        assert_eq!(find_claude_cli_in([a.clone()]), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn known_dirs_cover_the_native_installer() {
        let home = Path::new("/home/u");
        let dirs = claude_cli_dirs(Some(home));
        assert_eq!(dirs[0], home.join(".local").join("bin"));
        assert!(dirs.contains(&home.join(".claude").join("local")));
        if !cfg!(windows) {
            assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
            assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
        }
    }

    #[test]
    fn reads_the_bundled_version_beside_the_adapter() {
        let root = scratch("sdk");
        let script = root.join("node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        assert_eq!(bundled_cli_version(Some(&script)), CLAUDE_BUNDLED_CLI_VERSION, "nothing beside it yet");
        let sdk = root.join("node_modules/@anthropic-ai/claude-agent-sdk");
        std::fs::create_dir_all(&sdk).unwrap();
        std::fs::write(sdk.join("package.json"), br#"{"version":"0.3.290","claudeCodeVersion":"2.1.290"}"#).unwrap();
        assert_eq!(bundled_cli_version(Some(&script)), "2.1.290");
        assert_eq!(bundled_cli_version(None), CLAUDE_BUNDLED_CLI_VERSION);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The fallback constant follows the SDK the pinned adapter installs.
    /// Checked against the repo's node_modules when it is there.
    #[test]
    fn the_fallback_version_matches_the_installed_sdk() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../node_modules/@anthropic-ai/claude-agent-sdk/package.json");
        let Ok(text) = std::fs::read_to_string(manifest) else { return };
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["claudeCodeVersion"].as_str(), Some(CLAUDE_BUNDLED_CLI_VERSION));
    }
}
