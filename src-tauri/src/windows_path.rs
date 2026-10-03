//! The PATH a terminal's shell gets on Windows.
//!
//! portable-pty does not pass this process's PATH on. It builds one from
//! the registry, the machine's `Path` and then the user's, so a terminal
//! sees what an installer added since the app started. The machine's is a
//! `REG_EXPAND_SZ` full of `%SystemRoot%`, and it expands that from this
//! process's own environment. When that environment has no `SystemRoot`,
//! as when whatever launched the app passed a block of its own, the shell
//! gets `%SYSTEMROOT%\System32\OpenSSH\` as it stands: no directory, so no
//! `ssh`, no `where` and nothing else from System32. What lives under
//! Program Files or the home folder still works, which is why `git` and
//! `node` gave nothing away.
//!
//! So the PATH is built here instead, and given to the shell under one key:
//! the machine's, then the user's, then whatever this process had that
//! they do not, each directory once. Windows names are not case-sensitive,
//! so `Path` and `PATH` are one variable and `C:\Windows\System32` and
//! `c:\windows\system32\` one directory. A `%NAME%` is looked up in this
//! process, then in the registry's environment, and `SystemRoot` and
//! `windir` finally in the registry's record of where Windows is. An entry
//! that still has a `%NAME%` in it after all that names no directory, and
//! is left out.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use portable_pty::CommandBuilder;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

const MACHINE_ENV: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
const USER_ENV: &str = "Environment";
const WINDOWS_NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// Give `cmd` the PATH built from the registry and this process, and the
/// `SystemRoot` and `windir` this process lacks, if it lacks them: without
/// `SystemRoot` a child cannot even open a socket.
pub fn apply(cmd: &mut CommandBuilder) {
    let env = Registry::read();
    cmd.env("Path", path(&env));
    for name in ["SystemRoot", "windir"] {
        if std::env::var_os(name).is_none_or(|v| v.is_empty()) {
            if let Some(root) = env.windows_dir.as_deref() {
                cmd.env(name, root);
            }
        }
    }
}

/// The PATH [`apply`] gives a shell.
pub fn shell_path() -> OsString {
    path(&Registry::read())
}

fn path(env: &Registry) -> OsString {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    merge(&env.machine_path, &env.user_path, &inherited, |name| lookup(env, name, process_var))
}

/// What the registry says about the environment.
#[derive(Default)]
struct Registry {
    machine_path: String,
    user_path: String,
    /// Every other value of both keys, the user's after the machine's.
    vars: Vec<(String, String)>,
    /// Where Windows is, from `HKLM\...\Windows NT\CurrentVersion`.
    windows_dir: Option<String>,
}

impl Registry {
    fn read() -> Self {
        let mut out = Registry::default();
        for (hive, key) in [(HKEY_LOCAL_MACHINE, MACHINE_ENV), (HKEY_CURRENT_USER, USER_ENV)] {
            let Ok(k) = RegKey::predef(hive).open_subkey(key) else { continue };
            for (name, value) in k.enum_values().flatten() {
                // Read raw: a REG_EXPAND_SZ comes back with its %NAME%s in it.
                let Ok(text) = String::from_utf16(&wide(&value.bytes)) else { continue };
                if name.eq_ignore_ascii_case("path") {
                    if hive == HKEY_LOCAL_MACHINE {
                        out.machine_path = text;
                    } else {
                        out.user_path = text;
                    }
                } else {
                    out.vars.push((name, text));
                }
            }
        }
        out.windows_dir = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(WINDOWS_NT)
            .and_then(|k| k.get_value::<String, _>("SystemRoot"))
            .ok()
            .filter(|s| !s.is_empty());
        out
    }
}

/// A registry string's UTF-16, without the terminating NULs.
fn wide(bytes: &[u8]) -> Vec<u16> {
    let mut units: Vec<u16> = (0..bytes.len() / 2).map(|i| u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])).collect();
    while units.last() == Some(&0) {
        units.pop();
    }
    units
}

/// A variable of this process, if it is set to something.
fn process_var(name: &str) -> Option<String> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(|v| v.to_string_lossy().into_owned())
}

/// `%name%`'s value: `process`'s, else the registry's, else for
/// `SystemRoot` and `windir` the folder Windows is in.
fn lookup(env: &Registry, name: &str, process: impl Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(v) = process(name) {
        return Some(v);
    }
    // The user's value wins over the machine's, as it does for a new logon.
    if let Some((_, v)) = env.vars.iter().rev().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        // One level deep: `%A%` holding `%B%` is rare, and a loop is worse.
        return Some(expand(v, &process));
    }
    if name.eq_ignore_ascii_case("SystemRoot") || name.eq_ignore_ascii_case("windir") {
        return env.windows_dir.clone();
    }
    None
}

/// `s` with each `%NAME%` that `var` knows replaced. One it does not know
/// stays as it is, `%` signs and all.
fn expand(s: &str, var: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push('%');
            rest = after;
            continue;
        };
        let name = &after[..end];
        match var(name).filter(|_| !name.is_empty()) {
            Some(value) => out.push_str(&value),
            None => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The machine's directories, then the user's, then those of `inherited`
/// that neither has, each once. A directory still holding a `%NAME%` is
/// dropped, and so is an empty one.
fn merge(machine: &str, user: &str, inherited: &OsStr, var: impl Fn(&str) -> Option<String>) -> OsString {
    let expanded = [expand(machine, &var), expand(user, &var)];
    let from_registry = expanded.iter().flat_map(|p| std::env::split_paths(p));
    let mut seen: Vec<String> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in from_registry.chain(std::env::split_paths(inherited)) {
        let text = dir.to_string_lossy();
        if text.trim().is_empty() || unexpanded(&text) {
            continue;
        }
        let key = text.trim_end_matches(['\\', '/']).to_lowercase();
        if !seen.contains(&key) {
            seen.push(key);
            dirs.push(dir);
        }
    }
    // Every entry came out of a split on ';', so none holds one; a stray
    // '"' is all that could make joining fail.
    std::env::join_paths(&dirs).unwrap_or_else(|_| {
        let mut out = OsString::new();
        for (i, d) in dirs.iter().enumerate() {
            if i > 0 {
                out.push(";");
            }
            out.push(d.as_os_str());
        }
        out
    })
}

/// Whether `dir` still has a `%NAME%` in it.
fn unexpanded(dir: &str) -> bool {
    dir.find('%').is_some_and(|i| dir[i + 1..].find('%').is_some_and(|j| j > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACHINE: &str = r"%SystemRoot%\system32;%SystemRoot%;%SYSTEMROOT%\System32\OpenSSH\;C:\Program Files\Git\cmd";
    const USER: &str = r"%USERPROFILE%\.cargo\bin;C:\Users\me\AppData\Roaming\npm";

    fn vars(name: &str) -> Option<String> {
        match name.to_ascii_lowercase().as_str() {
            "systemroot" => Some(r"C:\WINDOWS".into()),
            "userprofile" => Some(r"C:\Users\me".into()),
            _ => None,
        }
    }

    fn dirs(path: &OsStr) -> Vec<String> {
        std::env::split_paths(path).map(|d| d.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn machine_then_user_then_what_the_process_had() {
        let path = merge(MACHINE, USER, OsStr::new(r"C:\repo\node_modules\.bin;C:\WINDOWS\system32"), vars);
        assert_eq!(
            dirs(&path),
            [
                r"C:\WINDOWS\system32",
                r"C:\WINDOWS",
                r"C:\WINDOWS\System32\OpenSSH\",
                r"C:\Program Files\Git\cmd",
                r"C:\Users\me\.cargo\bin",
                r"C:\Users\me\AppData\Roaming\npm",
                r"C:\repo\node_modules\.bin",
            ]
        );
    }

    #[test]
    fn a_directory_is_there_once_whatever_its_case_or_trailing_slash() {
        let path = merge(r"C:\Windows\System32;C:\Tools\", r"c:\windows\system32\;C:\TOOLS", OsStr::new(r"C:\tools;C:\Windows\System32"), vars);
        assert_eq!(dirs(&path), [r"C:\Windows\System32", r"C:\Tools\"]);
    }

    /// What broke `ssh`: nothing to expand `%SystemRoot%` with.
    #[test]
    fn what_cannot_be_expanded_is_left_out_not_passed_on() {
        let path = merge(MACHINE, USER, OsStr::new(""), |_| None);
        assert_eq!(dirs(&path), [r"C:\Program Files\Git\cmd", r"C:\Users\me\AppData\Roaming\npm"]);
    }

    #[test]
    fn systemroot_comes_from_where_windows_is_when_the_process_has_none() {
        let env = Registry {
            vars: vec![("ToolsRoot".into(), r"%SystemRoot%\tools".into())],
            windows_dir: Some(r"C:\WINDOWS".into()),
            ..Default::default()
        };
        let none = |_: &str| None;
        assert_eq!(lookup(&env, "SystemRoot", none).as_deref(), Some(r"C:\WINDOWS"));
        assert_eq!(lookup(&env, "WINDIR", none).as_deref(), Some(r"C:\WINDOWS"));
        assert_eq!(lookup(&env, "toolsroot", vars).as_deref(), Some(r"C:\WINDOWS\tools"));
        assert_eq!(lookup(&env, "NoSuchVar", none), None);
        // The process's own value comes first.
        assert_eq!(lookup(&env, "SystemRoot", |_| Some(r"D:\Win".into())).as_deref(), Some(r"D:\Win"));
    }

    #[test]
    fn expansion_leaves_what_it_does_not_know_and_lone_percent_signs() {
        assert_eq!(expand(r"%SystemRoot%\x;%NOPE%\y;50%;%%", vars), r"C:\WINDOWS\x;%NOPE%\y;50%;%%");
        assert!(unexpanded(r"%NOPE%\y"));
        assert!(!unexpanded(r"C:\50%"));
        assert!(!unexpanded(r"C:\a%%b"));
    }

    #[test]
    fn registry_strings_lose_their_terminating_nuls() {
        let bytes: Vec<u8> = "C:\\x\0\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(String::from_utf16(&wide(&bytes)).unwrap(), "C:\\x");
    }
}
