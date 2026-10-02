//! Paths as people and other programs read them.
//!
//! On Windows, `canonicalize` answers in the verbatim form (`\\?\C:\x`,
//! `\\?\UNC\server\share\x`). The file APIs take it, but shells, agents and
//! the human do not: PowerShell started there prompts
//! `PS Microsoft.PowerShell.Core\FileSystem::\\?\C:\x>`. A path that leaves
//! the app, or is kept to be handed out later, goes through [`plain`].

use std::path::{Path, PathBuf};

/// `\\?\C:\x` → `C:\x`, `\\?\UNC\server\share\x` → `\\server\share\x`;
/// anything else as it is.
pub fn plain(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\").filter(|r| is_drive(r)) {
        rest.to_string()
    } else {
        path.to_string()
    }
}

/// [`plain`], for a path.
pub fn plain_path(path: &Path) -> PathBuf {
    match path.to_str() {
        Some(s) => PathBuf::from(plain(s)),
        None => path.to_path_buf(),
    }
}

/// `C:` or `C:\…`: the only verbatim form with a plain twin besides UNC
/// (`\\?\Volume{…}` and the like have none).
fn is_drive(rest: &str) -> bool {
    let b = rest.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_stay() {
        assert_eq!(plain(r"C:\Users\x"), r"C:\Users\x");
        assert_eq!(plain(r"\\server\share\x"), r"\\server\share\x");
        assert_eq!(plain("/home/x/repo"), "/home/x/repo");
        assert_eq!(plain("relative/dir"), "relative/dir");
        assert_eq!(plain(""), "");
    }

    #[test]
    fn verbatim_drive_loses_its_prefix() {
        assert_eq!(plain(r"\\?\C:\x"), r"C:\x");
        assert_eq!(plain(r"\\?\C:\Users\iceba\work space"), r"C:\Users\iceba\work space");
        assert_eq!(plain(r"\\?\d:"), "d:");
    }

    #[test]
    fn verbatim_unc_becomes_unc() {
        assert_eq!(plain(r"\\?\UNC\srv\share\x"), r"\\srv\share\x");
    }

    #[test]
    fn verbatim_without_a_plain_twin_stays() {
        let volume = r"\\?\Volume{0b1c2d3e-0000-0000-0000-100000000000}\x";
        assert_eq!(plain(volume), volume);
        assert_eq!(plain(r"\\?\GLOBALROOT\Device"), r"\\?\GLOBALROOT\Device");
    }

    #[test]
    fn paths_too() {
        assert_eq!(plain_path(Path::new(r"\\?\C:\x")), PathBuf::from(r"C:\x"));
        assert_eq!(plain_path(Path::new("/tmp/x")), PathBuf::from("/tmp/x"));
    }
}
