//! The library: MCP servers and skills shared across agents.
//!
//! MCP servers live in the store and reach every conductor and lane
//! session through ACP's `session/new` — no agent config file is touched,
//! and whatever an agent has in its own config keeps working beside them.
//! For visibility, each agent's own MCP config is read (never written) and
//! can be imported into the library.
//!
//! Skills have no protocol channel: they are `SKILL.md` folders an agent
//! scans at startup. The library keeps one central folder under the app's
//! data directory and copies each skill into the agents' user skill
//! directories, leaving a marker file so only Orchestra's own copies are
//! ever refreshed or removed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orchestra_acp::{McpHttp, McpServerSpec, McpStdio};
use orchestra_agents::AgentKind;
use orchestra_store::Store;
use serde::{Deserialize, Serialize};

/// Store key holding the MCP server list (JSON array of [`McpServerDef`]).
const MCP_META: &str = "library:mcp";

/// File that marks a skill copy as Orchestra's, in the agent's skills dir.
const MARKER: &str = ".orchestra";

/// An MCP server as the library defines it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerDef {
    pub name: String,
    /// `stdio` or `http`.
    pub transport: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Agent ids this server is given to; empty means every agent.
    #[serde(default)]
    pub agents: Vec<String>,
}

fn yes() -> bool {
    true
}

impl McpServerDef {
    fn check(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("an MCP server needs a name".to_string());
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err("MCP server names use letters, digits, - and _".to_string());
        }
        match self.transport.as_str() {
            "stdio" if self.command.trim().is_empty() => Err("a stdio server needs a command".to_string()),
            "http" if self.url.trim().is_empty() => Err("an http server needs a URL".to_string()),
            "stdio" | "http" => Ok(()),
            other => Err(format!("unknown transport {other}")),
        }
    }

    fn applies_to(&self, agent: &str) -> bool {
        self.enabled && (self.agents.is_empty() || self.agents.iter().any(|a| a == agent))
    }

    fn to_spec(&self) -> McpServerSpec {
        if self.transport == "http" {
            McpServerSpec::Http(McpHttp {
                name: self.name.clone(),
                url: self.url.trim().to_string(),
                headers: self.headers.clone(),
            })
        } else {
            McpServerSpec::Stdio(McpStdio {
                name: self.name.clone(),
                command: self.command.trim().to_string(),
                args: self.args.clone(),
                env: self.env.clone(),
            })
        }
    }
}

/// Every MCP server in the library.
pub fn load_mcp(store: &Store) -> Result<Vec<McpServerDef>, String> {
    Ok(store
        .get_meta(MCP_META)
        .map_err(|e| e.to_string())?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

fn store_mcp(store: &Store, list: &[McpServerDef]) -> Result<(), String> {
    let json = serde_json::to_string(list).map_err(|e| e.to_string())?;
    store.set_meta(MCP_META, &json).map_err(|e| e.to_string())
}

/// Add or replace a server (by name).
pub fn save_mcp(store: &Store, mut def: McpServerDef) -> Result<Vec<McpServerDef>, String> {
    def.name = def.name.trim().to_string();
    def.check()?;
    let mut list = load_mcp(store)?;
    match list.iter_mut().find(|d| d.name == def.name) {
        Some(slot) => *slot = def,
        None => list.push(def),
    }
    store_mcp(store, &list)?;
    Ok(list)
}

pub fn delete_mcp(store: &Store, name: &str) -> Result<Vec<McpServerDef>, String> {
    let mut list = load_mcp(store)?;
    let before = list.len();
    list.retain(|d| d.name != name);
    if list.len() == before {
        return Err(format!("no MCP server {name}"));
    }
    store_mcp(store, &list)?;
    Ok(list)
}

/// The servers a session on `agent` gets from the library.
pub fn mcp_for_agent(store: &Store, agent: &str) -> Vec<McpServerSpec> {
    load_mcp(store)
        .unwrap_or_default()
        .iter()
        .filter(|d| d.applies_to(agent))
        .map(McpServerDef::to_spec)
        .collect()
}

/// An MCP server an agent has in its own configuration. Read only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMcp {
    pub agent: String,
    pub name: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    /// Which file it came from.
    pub source: String,
    /// Already in the library under this name.
    pub in_library: bool,
}

/// Where each agent keeps its user-level MCP config.
fn agent_mcp_file(home: &Path, kind: AgentKind) -> PathBuf {
    match kind {
        AgentKind::ClaudeCode => home.join(".claude.json"),
        AgentKind::Codex => home.join(".codex").join("config.toml"),
        AgentKind::Copilot => home.join(".copilot").join("mcp-config.json"),
        AgentKind::Antigravity => home.join(".gemini").join("config").join("mcp_config.json"),
    }
}

/// Every agent's own MCP servers, from their config files.
pub fn read_agent_mcp(store: &Store, home: &Path) -> Vec<AgentMcp> {
    let library = load_mcp(store).unwrap_or_default();
    let mut out = Vec::new();
    for kind in AgentKind::ALL {
        let file = agent_mcp_file(home, kind);
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let entries: Vec<(String, serde_json::Value)> = if kind == AgentKind::Codex {
            let table: toml::Table = match text.parse() {
                Ok(t) => t,
                Err(err) => {
                    tracing::warn!(path = %file.display(), %err, "could not parse agent MCP config");
                    continue;
                }
            };
            table
                .get("mcp_servers")
                .and_then(|v| v.as_table())
                .map(|servers| {
                    servers
                        .iter()
                        .filter_map(|(k, v)| serde_json::to_value(v).ok().map(|j| (k.clone(), j)))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            let json: serde_json::Value = match serde_json::from_str(&text) {
                Ok(j) => j,
                Err(err) => {
                    tracing::warn!(path = %file.display(), %err, "could not parse agent MCP config");
                    continue;
                }
            };
            json.get("mcpServers")
                .and_then(|v| v.as_object())
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default()
        };
        for (name, v) in entries {
            let url = ["url", "serverUrl", "httpUrl"]
                .iter()
                .find_map(|k| v.get(*k).and_then(|u| u.as_str()))
                .unwrap_or("")
                .to_string();
            let command = v.get("command").and_then(|c| c.as_str()).unwrap_or("").to_string();
            let args = v
                .get("args")
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            let transport = if !url.is_empty() { "http" } else { "stdio" };
            out.push(AgentMcp {
                agent: kind.id().to_string(),
                in_library: library.iter().any(|d| d.name == name),
                name,
                transport: transport.to_string(),
                command,
                args,
                url,
                source: file.display().to_string(),
            });
        }
    }
    out
}

/// Turn an agent's own server into a library entry (its env and headers
/// are not carried: secrets stay in the agent's file).
pub fn import_agent_mcp(store: &Store, home: &Path, agent: &str, name: &str) -> Result<Vec<McpServerDef>, String> {
    let found = read_agent_mcp(store, home)
        .into_iter()
        .find(|m| m.agent == agent && m.name == name)
        .ok_or_else(|| format!("{agent} has no MCP server {name}"))?;
    save_mcp(
        store,
        McpServerDef {
            name: found.name,
            transport: found.transport,
            command: found.command,
            args: found.args,
            env: Vec::new(),
            url: found.url,
            headers: Vec::new(),
            enabled: true,
            agents: Vec::new(),
        },
    )
}

// ----- skills -----

/// A skill in the library, and where it is installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: String,
    /// Agent ids holding Orchestra's copy.
    pub installed: Vec<String>,
    /// Agent ids holding a folder of the same name that is not ours.
    pub foreign: Vec<String>,
}

/// A skill found in an agent's own skills directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSkill {
    pub agent: String,
    pub name: String,
    pub description: String,
    pub path: String,
    /// Orchestra's copy, not the agent's own.
    pub managed: bool,
    /// A skill of this name is in the library.
    pub in_library: bool,
}

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    pub copied: Vec<String>,
    pub removed: Vec<String>,
    pub skipped: Vec<String>,
}

/// Where each agent scans for user-level skills.
pub fn agent_skill_dir(home: &Path, kind: AgentKind) -> PathBuf {
    match kind {
        AgentKind::ClaudeCode => home.join(".claude").join("skills"),
        AgentKind::Codex => home.join(".codex").join("skills"),
        AgentKind::Copilot => home.join(".copilot").join("skills"),
        AgentKind::Antigravity => home.join(".gemini").join("skills"),
    }
}

/// `name` and `description` from a SKILL.md front matter; the folder name
/// when the file has none.
fn read_front_matter(dir: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let mut name = String::new();
    let mut description = String::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim) == Some("---") {
        for line in lines {
            let line = line.trim();
            if line == "---" {
                break;
            }
            if let Some(v) = line.strip_prefix("name:") {
                name = v.trim().trim_matches(['"', '\'']).to_string();
            } else if let Some(v) = line.strip_prefix("description:") {
                description = v.trim().trim_matches(['"', '\'']).to_string();
            }
        }
    }
    if name.is_empty() {
        name = dir.file_name()?.to_string_lossy().into_owned();
    }
    Some((name, description))
}

/// Skill folders directly under `dir`: those with a SKILL.md.
fn skill_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("SKILL.md").is_file())
        .collect();
    out.sort();
    out
}

fn folder_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The library's skills, with where each is installed.
pub fn list_skills(skills_dir: &Path, home: &Path) -> Vec<SkillInfo> {
    skill_dirs(skills_dir)
        .into_iter()
        .map(|dir| {
            let folder = folder_name(&dir);
            let (name, description) = read_front_matter(&dir).unwrap_or((folder.clone(), String::new()));
            let mut installed = Vec::new();
            let mut foreign = Vec::new();
            for kind in AgentKind::ALL {
                let target = agent_skill_dir(home, kind).join(&folder);
                if target.join(MARKER).is_file() {
                    installed.push(kind.id().to_string());
                } else if target.is_dir() {
                    foreign.push(kind.id().to_string());
                }
            }
            SkillInfo {
                name: if name.is_empty() { folder } else { name },
                description,
                path: dir.display().to_string(),
                installed,
                foreign,
            }
        })
        .collect()
}

/// Skills in every agent's own directory.
pub fn read_agent_skills(skills_dir: &Path, home: &Path) -> Vec<AgentSkill> {
    let library: Vec<String> = skill_dirs(skills_dir).iter().map(|p| folder_name(p)).collect();
    let mut out = Vec::new();
    for kind in AgentKind::ALL {
        for dir in skill_dirs(&agent_skill_dir(home, kind)) {
            let folder = folder_name(&dir);
            let (name, description) = read_front_matter(&dir).unwrap_or((folder.clone(), String::new()));
            out.push(AgentSkill {
                agent: kind.id().to_string(),
                name,
                description,
                path: dir.display().to_string(),
                managed: dir.join(MARKER).is_file(),
                in_library: library.contains(&folder),
            });
        }
    }
    out
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Copy every library skill into the given agents' skill directories,
/// replacing Orchestra's earlier copies and removing copies of skills that
/// left the library. A folder that is not ours is left alone and reported.
pub fn sync_skills(skills_dir: &Path, home: &Path, agents: &[String]) -> Result<SyncReport, String> {
    let mut report = SyncReport::default();
    let sources = skill_dirs(skills_dir);
    let names: Vec<String> = sources.iter().map(|p| folder_name(p)).collect();
    for kind in AgentKind::ALL {
        if !agents.iter().any(|a| a == kind.id()) {
            continue;
        }
        let root = agent_skill_dir(home, kind);
        std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        // Copies of skills no longer in the library go.
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let dir = entry.path();
                let folder = folder_name(&dir);
                if dir.is_dir() && dir.join(MARKER).is_file() && !names.contains(&folder) {
                    std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                    report.removed.push(format!("{}/{folder}", kind.id()));
                }
            }
        }
        for src in &sources {
            let folder = folder_name(src);
            let target = root.join(&folder);
            if target.is_dir() && !target.join(MARKER).is_file() {
                report.skipped.push(format!("{}/{folder}", kind.id()));
                continue;
            }
            if target.is_dir() {
                std::fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            }
            copy_dir(src, &target).map_err(|e| format!("{}: {e}", target.display()))?;
            std::fs::write(target.join(MARKER), "copied by Orchestra; edit the library copy instead\n")
                .map_err(|e| format!("{}: {e}", target.display()))?;
            report.copied.push(format!("{}/{folder}", kind.id()));
        }
    }
    Ok(report)
}

fn check_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("a skill name is 1 to 64 characters".to_string());
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err("skill names use lowercase letters, digits and -".to_string());
    }
    Ok(())
}

/// Start a skill in the library: a folder with a SKILL.md to fill in.
pub fn create_skill(skills_dir: &Path, name: &str, description: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    check_skill_name(name)?;
    let dir = skills_dir.join(name);
    if dir.exists() {
        return Err(format!("skill {name} already exists"));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let description = description.trim().replace('\n', " ");
    let body = format!(
        "---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\n<!-- When to use this skill, and the steps to follow. Keep it to what an agent needs. -->\n"
    );
    std::fs::write(dir.join("SKILL.md"), body).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Copy a skill from an agent's own directory into the library.
pub fn import_skill(skills_dir: &Path, home: &Path, agent: &str, folder: &str) -> Result<PathBuf, String> {
    let kind = AgentKind::parse(agent).ok_or_else(|| format!("unknown agent {agent}"))?;
    check_skill_name(folder).or_else(|_| {
        // Foreign folders may be named freely; only refuse path tricks.
        if folder.contains(['/', '\\', ':']) || folder.starts_with('.') {
            Err(format!("cannot import {folder}"))
        } else {
            Ok(())
        }
    })?;
    let src = agent_skill_dir(home, kind).join(folder);
    if !src.join("SKILL.md").is_file() {
        return Err(format!("{agent} has no skill {folder}"));
    }
    let dst = skills_dir.join(folder);
    if dst.exists() {
        return Err(format!("the library already has {folder}"));
    }
    copy_dir(&src, &dst).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(dst.join(MARKER));
    Ok(dst)
}

/// Remove a skill from the library and Orchestra's copies of it everywhere.
pub fn remove_skill(skills_dir: &Path, home: &Path, folder: &str) -> Result<Vec<String>, String> {
    if folder.contains(['/', '\\', ':']) || folder.starts_with('.') || folder.is_empty() {
        return Err(format!("cannot remove {folder}"));
    }
    let dir = skills_dir.join(folder);
    if !dir.is_dir() {
        return Err(format!("no skill {folder}"));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut removed = Vec::new();
    for kind in AgentKind::ALL {
        let target = agent_skill_dir(home, kind).join(folder);
        if target.join(MARKER).is_file() {
            std::fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
            removed.push(kind.id().to_string());
        }
    }
    Ok(removed)
}

/// Where the library's skills live: `<app data>/skills`.
pub fn skills_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("skills")
}

/// Show a folder or file in the system file manager.
pub fn reveal(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("no such path: {}", path.display()));
    }
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("explorer");
        c.arg(path);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Presence of each agent's skill directory, for the library page.
pub fn skill_dirs_of(home: &Path) -> BTreeMap<String, String> {
    AgentKind::ALL
        .iter()
        .map(|k| (k.id().to_string(), agent_skill_dir(home, *k).display().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orchestra-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn mcp_servers_round_trip_and_filter_by_agent() {
        let store = Store::in_memory().unwrap();
        let def = McpServerDef {
            name: "docs".into(),
            transport: "stdio".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "docs-mcp".into()],
            env: vec![("KEY".into(), "v".into())],
            url: String::new(),
            headers: Vec::new(),
            enabled: true,
            agents: vec!["codex".into()],
        };
        save_mcp(&store, def.clone()).unwrap();
        save_mcp(&store, McpServerDef { name: "web".into(), transport: "http".into(), url: "http://x".into(), agents: Vec::new(), ..def.clone() }).unwrap();
        assert_eq!(load_mcp(&store).unwrap().len(), 2);
        assert_eq!(mcp_for_agent(&store, "codex").len(), 2);
        assert_eq!(mcp_for_agent(&store, "claude_code").len(), 1, "docs is for codex only");
        assert!(save_mcp(&store, McpServerDef { name: "bad name".into(), ..def.clone() }).is_err());
        assert!(save_mcp(&store, McpServerDef { name: "x".into(), command: String::new(), ..def.clone() }).is_err());
        delete_mcp(&store, "docs").unwrap();
        assert!(delete_mcp(&store, "docs").is_err());
        assert_eq!(load_mcp(&store).unwrap().len(), 1);
    }

    #[test]
    fn skills_sync_copy_and_remove_only_marked_folders() {
        let root = temp("skills");
        let lib = root.join("lib");
        let home = root.join("home");
        std::fs::create_dir_all(&lib).unwrap();
        create_skill(&lib, "review", "review a diff").unwrap();
        assert!(create_skill(&lib, "review", "").is_err());
        assert!(create_skill(&lib, "Bad Name", "").is_err());
        // A skill of the agent's own, same name, must survive a sync.
        let own = agent_skill_dir(&home, AgentKind::Codex).join("review");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("SKILL.md"), "---\nname: review\ndescription: mine\n---\n").unwrap();

        let agents = vec!["claude_code".to_string(), "codex".to_string()];
        let report = sync_skills(&lib, &home, &agents).unwrap();
        assert_eq!(report.copied, vec!["claude_code/review"]);
        assert_eq!(report.skipped, vec!["codex/review"]);
        let copy = agent_skill_dir(&home, AgentKind::ClaudeCode).join("review");
        assert!(copy.join("SKILL.md").is_file() && copy.join(MARKER).is_file());

        let list = list_skills(&lib, &home);
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].name.as_str(), list[0].description.as_str()), ("review", "review a diff"));
        assert_eq!(list[0].installed, vec!["claude_code"]);
        assert_eq!(list[0].foreign, vec!["codex"]);

        let found = read_agent_skills(&lib, &home);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|s| s.agent == "claude_code" && s.managed && s.in_library));
        assert!(found.iter().any(|s| s.agent == "codex" && !s.managed && s.in_library));

        // Removing takes the library copy and ours, never the agent's own.
        let removed = remove_skill(&lib, &home, "review").unwrap();
        assert_eq!(removed, vec!["claude_code"]);
        assert!(!copy.exists() && own.join("SKILL.md").is_file());

        // Importing brings the agent's own skill into the library, unmarked.
        let dst = import_skill(&lib, &home, "codex", "review").unwrap();
        assert!(dst.join("SKILL.md").is_file() && !dst.join(MARKER).exists());
        assert!(import_skill(&lib, &home, "codex", "review").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn agent_mcp_configs_are_read_in_each_format() {
        let home = temp("mcp-home");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".copilot")).unwrap();
        std::fs::create_dir_all(home.join(".gemini/config")).unwrap();
        std::fs::write(home.join(".claude.json"), r#"{"mcpServers":{"devterm":{"type":"stdio","command":"node","args":["m.js"]}}}"#).unwrap();
        std::fs::write(home.join(".codex/config.toml"), "[mcp_servers.docs]\ncommand = \"npx\"\nargs = [\"-y\", \"docs\"]\n").unwrap();
        std::fs::write(home.join(".copilot/mcp-config.json"), r#"{"mcpServers":{"gh":{"type":"http","url":"https://api.githubcopilot.com/mcp/"}}}"#).unwrap();
        std::fs::write(home.join(".gemini/config/mcp_config.json"), "").unwrap();
        let store = Store::in_memory().unwrap();
        let found = read_agent_mcp(&store, &home);
        let names: Vec<(String, String, String)> = found.iter().map(|m| (m.agent.clone(), m.name.clone(), m.transport.clone())).collect();
        assert_eq!(
            names,
            vec![
                ("claude_code".to_string(), "devterm".to_string(), "stdio".to_string()),
                ("codex".to_string(), "docs".to_string(), "stdio".to_string()),
                ("copilot".to_string(), "gh".to_string(), "http".to_string()),
            ]
        );
        import_agent_mcp(&store, &home, "codex", "docs").unwrap();
        assert!(read_agent_mcp(&store, &home).iter().any(|m| m.name == "docs" && m.in_library));
        let _ = std::fs::remove_dir_all(&home);
    }
}
