//! The shared libraries divixi-server needs from the machine it runs on.
//!
//! The release binary is the app on Tauri's mock runtime (the `server`
//! feature), and Tauri links its Linux GUI stack whichever runtime runs:
//! the binary's ELF `NEEDED` entries name `libwebkit2gtk-4.1.so.0`,
//! `libgtk-3.so.0`, `libgdk-3.so.0`, `libgdk_pixbuf-2.0.so.0` and GLib.
//! Nothing of it is used without a window, but the loader does not know
//! that: on a headless box without them the binary never reaches `main`,
//! and all there is to see is the loader's one line on stderr:
//!
//! ```text
//! divixi-server: error while loading shared libraries: libwebkit2gtk-4.1.so.0: cannot open shared object file: No such file or directory
//! ```
//!
//! Installing system packages needs root, which the app does not ask for.
//! So what is done here is finding out which libraries are missing (`ldd`,
//! and the loader's own line when the binary is run) and naming the package
//! and the one command that puts them there.

use serde::Serialize;

/// The machine's package manager, as far as its command names say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pm {
    Apt,
    Dnf,
    Pacman,
}

impl Pm {
    /// What the probe's `pm=` line said: the first of these found on PATH.
    pub fn parse(s: &str) -> Option<Pm> {
        match s.trim() {
            "apt" | "apt-get" => Some(Pm::Apt),
            "dnf" | "yum" => Some(Pm::Dnf),
            "pacman" => Some(Pm::Pacman),
            _ => None,
        }
    }

    fn install(self, packages: &[&str]) -> String {
        let list = packages.join(" ");
        match self {
            Pm::Apt => format!("sudo apt install -y {list}"),
            Pm::Dnf => format!("sudo dnf install -y {list}"),
            Pm::Pacman => format!("sudo pacman -S --needed {list}"),
        }
    }
}

/// One library the binary links, and the package that has it.
struct Lib {
    soname: &'static str,
    apt: &'static str,
    dnf: &'static str,
    pacman: &'static str,
    /// The WebKitGTK package depends on it, so naming that package is enough.
    with_webkit: bool,
}

const WEBKIT: &str = "libwebkit2gtk-4.1.so.0";

/// The libraries in the release binary's `NEEDED` list beyond glibc, and
/// the two WebKitGTK itself needs most often.
const LIBS: &[Lib] = &[
    Lib { soname: WEBKIT, apt: "libwebkit2gtk-4.1-0", dnf: "webkit2gtk4.1", pacman: "webkit2gtk-4.1", with_webkit: false },
    Lib { soname: "libjavascriptcoregtk-4.1.so.0", apt: "libjavascriptcoregtk-4.1-0", dnf: "javascriptcoregtk4.1", pacman: "webkit2gtk-4.1", with_webkit: true },
    Lib { soname: "libsoup-3.0.so.0", apt: "libsoup-3.0-0", dnf: "libsoup3", pacman: "libsoup3", with_webkit: true },
    Lib { soname: "libgtk-3.so.0", apt: "libgtk-3-0", dnf: "gtk3", pacman: "gtk3", with_webkit: true },
    Lib { soname: "libgdk-3.so.0", apt: "libgtk-3-0", dnf: "gtk3", pacman: "gtk3", with_webkit: true },
    Lib { soname: "libgdk_pixbuf-2.0.so.0", apt: "libgdk-pixbuf-2.0-0", dnf: "gdk-pixbuf2", pacman: "gdk-pixbuf2", with_webkit: true },
    Lib { soname: "libglib-2.0.so.0", apt: "libglib2.0-0", dnf: "glib2", pacman: "glib2", with_webkit: true },
    Lib { soname: "libgobject-2.0.so.0", apt: "libglib2.0-0", dnf: "glib2", pacman: "glib2", with_webkit: true },
    Lib { soname: "libgio-2.0.so.0", apt: "libglib2.0-0", dnf: "glib2", pacman: "glib2", with_webkit: true },
    Lib { soname: "libgcc_s.so.1", apt: "libgcc-s1", dnf: "libgcc", pacman: "gcc-libs", with_webkit: false },
];

/// The library a line of `ldd` or of the loader names as missing.
///
/// `ldd` says `\tlibwebkit2gtk-4.1.so.0 => not found`; the loader says
/// `<path>: error while loading shared libraries: <soname>: cannot open
/// shared object file: …`. Neither is translated (they come from ld.so,
/// which has no message catalogue), so the words can be matched.
pub fn soname(line: &str) -> Option<String> {
    const LOADER: &str = "error while loading shared libraries: ";
    let name = if let Some(at) = line.find(LOADER) {
        let rest = &line[at + LOADER.len()..];
        rest.split(':').next().unwrap_or_default()
    } else if let Some((name, rest)) = line.split_once("=>") {
        if rest.trim() != "not found" {
            return None;
        }
        name
    } else {
        return None;
    };
    let name = name.trim();
    (!name.is_empty() && !name.contains(char::is_whitespace)).then(|| name.to_string())
}

/// Every library the lines name as missing, once each, in the order seen.
pub fn missing<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in lines.into_iter().filter_map(soname) {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// What is missing there, and what to run to put it there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Libs {
    /// The sonames, as the loader names them.
    pub missing: Vec<String>,
    /// The packages that have them, for the machine's package manager.
    /// Empty when that manager is not known.
    pub packages: Vec<String>,
    /// The one line to run there. `None` when the package manager, or every
    /// library, is one this does not know.
    pub command: Option<String>,
}

impl Libs {
    pub fn new(missing: Vec<String>, pm: Option<Pm>) -> Libs {
        let known: Vec<&Lib> = missing.iter().filter_map(|m| LIBS.iter().find(|l| l.soname == m)).collect();
        let webkit = known.iter().any(|l| l.soname == WEBKIT);
        let mut packages: Vec<&str> = Vec::new();
        if let Some(pm) = pm {
            for lib in known.iter().filter(|l| !(webkit && l.with_webkit)) {
                let name = match pm {
                    Pm::Apt => lib.apt,
                    Pm::Dnf => lib.dnf,
                    Pm::Pacman => lib.pacman,
                };
                if !packages.contains(&name) {
                    packages.push(name);
                }
            }
        }
        let command = match pm {
            Some(pm) if !packages.is_empty() => Some(pm.install(&packages)),
            _ => None,
        };
        Libs { missing, packages: packages.into_iter().map(String::from).collect(), command }
    }

    /// The same, as one line for an error string or a log.
    pub fn message(&self) -> String {
        let what = self.missing.join(", ");
        match &self.command {
            Some(cmd) => format!("divixi-server cannot start on that machine: {what} is missing. Run this there, then try again: {cmd}"),
            None => format!("divixi-server cannot start on that machine: {what} is missing. Install the package that has it (docs/divixi-server.md), then try again"),
        }
    }
}

/// Shell lines that print `lib=<line>` for every line of `ldd` or of the
/// loader that names a missing library of the file in `var` (a shell
/// variable, such as `"$b"`).
///
/// `help` is run as well as `ldd`, because `ldd` may not be there and the
/// loader's own line is what decides whether the binary starts. With
/// everything in place it prints its usage, which goes nowhere.
pub fn script(var: &str) -> String {
    format!(
        r#"if command -v ldd >/dev/null 2>&1; then LC_ALL=C ldd {var} 2>&1 | grep 'not found' | sed 's/^/lib=/'; fi
LC_ALL=C {var} help 2>&1 >/dev/null </dev/null | grep 'error while loading shared libraries' | sed 's/^/lib=/'
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loader's line, as glibc prints it for the release binary.
    const LOADER: &str = "/home/u/.divixi/server/current/divixi-server: error while loading shared libraries: libwebkit2gtk-4.1.so.0: cannot open shared object file: No such file or directory";

    #[test]
    fn the_loader_line_names_the_library() {
        assert_eq!(soname(LOADER).as_deref(), Some("libwebkit2gtk-4.1.so.0"));
        // A path with spaces and colons before it does not confuse it.
        let odd = "/home/my user/a:b/divixi-server: error while loading shared libraries: libgtk-3.so.0: cannot open shared object file: No such file or directory";
        assert_eq!(soname(odd).as_deref(), Some("libgtk-3.so.0"));
    }

    #[test]
    fn ldd_lines_name_only_what_is_not_found() {
        assert_eq!(soname("\tlibwebkit2gtk-4.1.so.0 => not found").as_deref(), Some("libwebkit2gtk-4.1.so.0"));
        assert_eq!(soname("\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f0000000000)"), None);
        assert_eq!(soname("\tlinux-vdso.so.1 (0x00007ffd00000000)"), None);
        assert_eq!(soname("\t/lib64/ld-linux-x86-64.so.2 (0x00007f0000000000)"), None);
        assert_eq!(soname(""), None);
        assert_eq!(soname("not a dynamic executable"), None);
    }

    #[test]
    fn the_probe_lines_come_back_once_each() {
        let out = [
            "\tlibgdk-3.so.0 => not found",
            "\tlibwebkit2gtk-4.1.so.0 => not found",
            "\tlibgtk-3.so.0 => not found",
            LOADER,
        ];
        assert_eq!(missing(out), ["libgdk-3.so.0", "libwebkit2gtk-4.1.so.0", "libgtk-3.so.0"]);
        assert!(missing(["\tlibc.so.6 => /lib/libc.so.6 (0x0)"]).is_empty());
    }

    #[test]
    fn webkit_missing_is_one_package_to_install() {
        // A fresh Ubuntu server: ldd lists webkit and the GTK it would bring.
        let gone = missing(["\tlibgdk-3.so.0 => not found", "\tlibwebkit2gtk-4.1.so.0 => not found", "\tlibgtk-3.so.0 => not found", "\tlibgdk_pixbuf-2.0.so.0 => not found"]);
        let l = Libs::new(gone.clone(), Some(Pm::Apt));
        assert_eq!(l.packages, ["libwebkit2gtk-4.1-0"]);
        assert_eq!(l.command.as_deref(), Some("sudo apt install -y libwebkit2gtk-4.1-0"));
        assert_eq!(l.missing, gone);
        assert_eq!(Libs::new(gone.clone(), Some(Pm::Dnf)).command.as_deref(), Some("sudo dnf install -y webkit2gtk4.1"));
        assert_eq!(Libs::new(gone, Some(Pm::Pacman)).command.as_deref(), Some("sudo pacman -S --needed webkit2gtk-4.1"));
    }

    #[test]
    fn without_webkit_missing_each_library_names_its_own_package() {
        let l = Libs::new(vec!["libgtk-3.so.0".into(), "libgdk-3.so.0".into(), "libglib-2.0.so.0".into()], Some(Pm::Apt));
        assert_eq!(l.packages, ["libgtk-3-0", "libglib2.0-0"], "gtk and gdk are one package");
        assert_eq!(l.command.as_deref(), Some("sudo apt install -y libgtk-3-0 libglib2.0-0"));
    }

    #[test]
    fn an_unknown_manager_or_library_gives_no_command() {
        let l = Libs::new(vec![WEBKIT.into()], None);
        assert!(l.packages.is_empty());
        assert_eq!(l.command, None);
        assert!(l.message().contains("libwebkit2gtk-4.1.so.0 is missing"));
        assert!(l.message().contains("docs/divixi-server.md"));
        let l = Libs::new(vec!["libsomething.so.9".into()], Some(Pm::Apt));
        assert_eq!(l.command, None);
        assert_eq!(l.missing, ["libsomething.so.9"]);
    }

    #[test]
    fn the_message_carries_the_command() {
        let l = Libs::new(vec![WEBKIT.into()], Some(Pm::Apt));
        assert_eq!(
            l.message(),
            "divixi-server cannot start on that machine: libwebkit2gtk-4.1.so.0 is missing. Run this there, then try again: sudo apt install -y libwebkit2gtk-4.1-0"
        );
    }

    #[test]
    fn package_managers_are_read_from_their_command_names() {
        assert_eq!(Pm::parse("apt-get"), Some(Pm::Apt));
        assert_eq!(Pm::parse("yum"), Some(Pm::Dnf));
        assert_eq!(Pm::parse("pacman\n"), Some(Pm::Pacman));
        assert_eq!(Pm::parse("zypper"), None);
        assert_eq!(Pm::parse(""), None);
    }

    #[test]
    fn the_script_reads_the_variable_it_is_given() {
        let s = script("\"$b\"");
        assert!(s.contains("ldd \"$b\""));
        assert!(s.contains("\"$b\" help"));
        assert!(s.lines().all(|l| l.contains("sed 's/^/lib=/'")));
    }
}
