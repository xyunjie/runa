//! Where runtimes live on macOS, Linux and Windows, and who owns a given installation.
//!
//! Everything here is a pure function of an [`Env`] (home dir + relevant environment
//! variables), so the layouts of every platform can be unit-tested on any machine with
//! fake directories. [`Env::current`] reads the real machine.

use std::path::{Path, PathBuf};

use crate::platform::{path_key, path_key_for};

/// Directories and environment variables that decide where version managers keep things.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// User home (`$HOME` / `%USERPROFILE%`).
    pub home: PathBuf,
    /// `%APPDATA%` (Windows roaming app data).
    pub appdata: Option<PathBuf>,
    /// `%LOCALAPPDATA%`.
    pub local_appdata: Option<PathBuf>,
    /// `%ProgramFiles%`, `%ProgramFiles(x86)%`.
    pub program_files: Vec<PathBuf>,
    /// `%ProgramData%`.
    pub program_data: Option<PathBuf>,
    /// `$XDG_DATA_HOME` (defaults to `~/.local/share`).
    pub xdg_data_home: Option<PathBuf>,
    /// Version-manager overrides: NVM_DIR, NVM_HOME, FNM_DIR, PYENV_ROOT, SDKMAN_DIR,
    /// ASDF_DATA_DIR, MISE_DATA_DIR, SCOOP, SCOOP_GLOBAL, HOMEBREW_CELLAR, ChocolateyInstall.
    pub vars: Vec<(String, PathBuf)>,
}

/// Environment variables consulted (see [`Env::vars`]).
pub const ENV_VARS: &[&str] = &[
    "NVM_DIR",
    "NVM_HOME",
    "FNM_DIR",
    "PYENV_ROOT",
    "SDKMAN_DIR",
    "ASDF_DATA_DIR",
    "MISE_DATA_DIR",
    "SCOOP",
    "SCOOP_GLOBAL",
    "HOMEBREW_CELLAR",
    "ChocolateyInstall",
];

impl Env {
    pub fn current() -> Self {
        let get = |k: &str| {
            std::env::var_os(k)
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
        };
        let home = dirs::home_dir()
            .or_else(|| get("USERPROFILE"))
            .or_else(|| get("HOME"))
            .unwrap_or_default();
        Env {
            home,
            appdata: get("APPDATA"),
            local_appdata: get("LOCALAPPDATA"),
            program_files: ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
                .iter()
                .filter_map(|k| get(k))
                .fold(Vec::new(), |mut v, p| {
                    if !v.iter().any(|q| crate::platform::same_path(q, &p)) {
                        v.push(p);
                    }
                    v
                }),
            program_data: get("ProgramData"),
            xdg_data_home: get("XDG_DATA_HOME"),
            vars: ENV_VARS
                .iter()
                .filter_map(|k| get(k).map(|p| (k.to_string(), p)))
                .collect(),
        }
    }

    pub fn var(&self, k: &str) -> Option<&Path> {
        self.vars
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, p)| p.as_path())
    }

    fn data_home(&self) -> PathBuf {
        self.xdg_data_home
            .clone()
            .unwrap_or_else(|| self.home.join(".local").join("share"))
    }

    /// Program Files dirs, with the default `C:\Program Files` on Windows when unset.
    fn program_files(&self) -> Vec<PathBuf> {
        if self.program_files.is_empty() && cfg!(windows) {
            vec![PathBuf::from(r"C:\Program Files")]
        } else {
            self.program_files.clone()
        }
    }

    /// Manager data dirs: `(provider, dir)`; an env override replaces the default.
    pub fn manager_dirs(&self) -> Vec<(&'static str, PathBuf)> {
        let h = &self.home;
        let or = |k: &str, d: PathBuf| self.var(k).map(Path::to_path_buf).unwrap_or(d);
        let mut v = vec![
            ("nvm", or("NVM_DIR", h.join(".nvm"))),
            ("sdkman", or("SDKMAN_DIR", h.join(".sdkman"))),
            ("asdf", or("ASDF_DATA_DIR", h.join(".asdf"))),
        ];
        // pyenv / pyenv-win: PYENV_ROOT points at `.pyenv` (unix) or `.pyenv\pyenv-win`
        let pyenv_win = h.join(".pyenv").join("pyenv-win");
        match self.var("PYENV_ROOT") {
            Some(r) if is_pyenv_win_root(r) => {
                v.push(("pyenv-win", r.to_path_buf()));
                v.push(("pyenv", h.join(".pyenv")));
            }
            Some(r) => {
                v.push(("pyenv", r.to_path_buf()));
                v.push(("pyenv-win", pyenv_win));
            }
            None => {
                v.push(("pyenv", h.join(".pyenv")));
                v.push(("pyenv-win", pyenv_win));
            }
        }
        // fnm: FNM_DIR, else XDG data / macOS Application Support / %APPDATA%\fnm
        match self.var("FNM_DIR") {
            Some(d) => v.push(("fnm", d.to_path_buf())),
            None => {
                v.push(("fnm", self.data_home().join("fnm")));
                v.push((
                    "fnm",
                    h.join("Library").join("Application Support").join("fnm"),
                ));
                if let Some(a) = &self.appdata {
                    v.push(("fnm", a.join("fnm")));
                }
                v.push(("fnm", h.join(".fnm")));
            }
        }
        // nvm-windows: NVM_HOME, else %APPDATA%\nvm
        match (self.var("NVM_HOME"), &self.appdata) {
            (Some(d), _) => v.push(("nvm-windows", d.to_path_buf())),
            (None, a) => {
                // nvm-windows <= 1.1.11 used %APPDATA%\nvm, newer installers %LOCALAPPDATA%\nvm
                if let Some(a) = a {
                    v.push(("nvm-windows", a.join("nvm")));
                }
                if let Some(l) = &self.local_appdata {
                    v.push(("nvm-windows", l.join("nvm")));
                }
            }
        }
        // mise: MISE_DATA_DIR, else XDG data (or %LOCALAPPDATA%\mise)
        match self.var("MISE_DATA_DIR") {
            Some(d) => v.push(("mise", d.to_path_buf())),
            None => {
                v.push(("mise", self.data_home().join("mise")));
                if let Some(l) = &self.local_appdata {
                    v.push(("mise", l.join("mise")));
                }
            }
        }
        // scoop: per user and global
        v.push(("scoop", or("SCOOP", h.join("scoop"))));
        if let Some(g) = self.var("SCOOP_GLOBAL") {
            v.push(("scoop-global", g.to_path_buf()));
        } else if let Some(pd) = &self.program_data {
            v.push(("scoop-global", pd.join("scoop")));
        }
        v
    }

    /// Data dirs of `provider` (see [`Env::manager_dirs`]), most specific first.
    pub fn manager_dir(&self, provider: &str) -> Vec<PathBuf> {
        self.manager_dirs()
            .into_iter()
            .filter(|(p, _)| *p == provider)
            .map(|(_, d)| d)
            .collect()
    }

    /// Homebrew Cellars (macOS arm/intel, Linuxbrew system and per-user).
    pub fn brew_cellars(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self
            .var("HOMEBREW_CELLAR")
            .map(Path::to_path_buf)
            .into_iter()
            .collect();
        v.extend(
            [
                "/opt/homebrew/Cellar",
                "/usr/local/Cellar",
                "/home/linuxbrew/.linuxbrew/Cellar",
            ]
            .iter()
            .map(PathBuf::from),
        );
        v.push(self.home.join(".linuxbrew").join("Cellar"));
        dedup(v)
    }
}

fn is_pyenv_win_root(p: &Path) -> bool {
    path_key_for(p, true).ends_with("/pyenv-win")
}

fn dedup(v: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in v {
        if !out.iter().any(|q| path_key(q) == path_key(&p)) {
            out.push(p);
        }
    }
    out
}

/// A directory holding one runtime per child: `(dir, provider label, suffix from the
/// child to the runtime home)`. Suffix segments are joined with [`Path::join`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    pub dir: PathBuf,
    pub label: &'static str,
    pub suffix: &'static [&'static str],
    /// Only children whose name passes this filter (scoop apps, `~/sdk/go*`).
    pub filter: ChildFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildFilter {
    Any,
    /// Children starting with this prefix.
    Prefix(&'static str),
}

impl ChildFilter {
    pub fn accepts(self, name: &str) -> bool {
        match self {
            ChildFilter::Any => true,
            ChildFilter::Prefix(p) => name.starts_with(p),
        }
    }
}

fn store(dir: PathBuf, label: &'static str, suffix: &'static [&'static str]) -> Store {
    Store {
        dir,
        label,
        suffix,
        filter: ChildFilter::Any,
    }
}

/// Label used for runtimes found in vendor / OS default locations.
pub const WELL_KNOWN: &str = "常见目录";

/// Version-manager stores and vendor directories containing one runtime per child.
pub fn stores(env: &Env, tool: &str) -> Vec<Store> {
    let mut v = Vec::new();
    let md = |p: &str| env.manager_dir(p);
    match tool {
        "node" => {
            for d in md("nvm") {
                v.push(store(d.join("versions").join("node"), "nvm", &[]));
            }
            for d in md("asdf") {
                v.push(store(d.join("installs").join("nodejs"), "asdf", &[]));
            }
            for d in md("mise") {
                v.push(store(d.join("installs").join("node"), "mise", &[]));
            }
            for d in md("fnm") {
                v.push(store(d.join("node-versions"), "fnm", &["installation"]));
            }
            for d in md("nvm-windows") {
                v.push(Store {
                    filter: ChildFilter::Prefix("v"),
                    ..store(d, "nvm-windows", &[])
                });
            }
        }
        "python" => {
            for d in md("pyenv") {
                v.push(store(d.join("versions"), "pyenv", &[]));
            }
            for d in md("pyenv-win") {
                v.push(store(d.join("versions"), "pyenv-win", &[]));
            }
            for d in md("asdf") {
                v.push(store(d.join("installs").join("python"), "asdf", &[]));
            }
            for d in md("mise") {
                v.push(store(d.join("installs").join("python"), "mise", &[]));
            }
            if let Some(l) = &env.local_appdata {
                v.push(store(l.join("Programs").join("Python"), WELL_KNOWN, &[]));
            }
        }
        "java" => {
            for d in md("sdkman") {
                v.push(store(d.join("candidates").join("java"), "sdkman", &[]));
            }
            for d in md("asdf") {
                v.push(store(d.join("installs").join("java"), "asdf", &[]));
            }
            for d in md("mise") {
                v.push(store(d.join("installs").join("java"), "mise", &[]));
            }
            v.push(store(
                env.home
                    .join("Library")
                    .join("Java")
                    .join("JavaVirtualMachines"),
                WELL_KNOWN,
                &["Contents", "Home"],
            ));
            // IntelliJ / `jdk` downloads on Linux and Windows
            v.push(store(env.home.join(".jdks"), WELL_KNOWN, &[]));
        }
        "go" => {
            for d in md("sdkman") {
                v.push(store(d.join("candidates").join("go"), "sdkman", &[]));
            }
            for d in md("asdf") {
                v.push(store(d.join("installs").join("golang"), "asdf", &["go"]));
            }
            for d in md("mise") {
                v.push(store(d.join("installs").join("go"), "mise", &[]));
            }
            // `go install golang.org/dl/goX.Y` -> ~/sdk/goX.Y
            v.push(Store {
                filter: ChildFilter::Prefix("go"),
                ..store(env.home.join("sdk"), WELL_KNOWN, &[])
            });
        }
        _ => {}
    }
    // scoop: apps/<app>/<version>
    for (label, d) in env
        .manager_dirs()
        .into_iter()
        .filter(|(p, _)| p.starts_with("scoop"))
    {
        let apps = d.join("apps");
        for app in scoop_apps(tool) {
            v.push(store(apps.join(app), label, &[]));
        }
    }
    v
}

/// Scoop app names providing `tool` (main and versions buckets / java bucket).
fn scoop_apps(tool: &str) -> &'static [&'static str] {
    match tool {
        "node" => &["nodejs", "nodejs-lts"],
        "python" => &[
            "python",
            "python313",
            "python312",
            "python311",
            "python310",
            "python39",
        ],
        "java" => &[
            "openjdk",
            "openjdk21",
            "openjdk17",
            "openjdk11",
            "temurin-jdk",
            "temurin21-jdk",
            "temurin17-jdk",
            "temurin11-jdk",
            "temurin8-jdk",
            "zulu-jdk",
            "corretto-jdk",
            "microsoft-jdk",
            "liberica-jdk",
        ],
        "go" => &["go"],
        _ => &[],
    }
}

/// Parent directories whose children are runtime homes (vendor installers / OS packages).
/// `(dir, suffix)`.
pub fn vendor_roots(env: &Env, tool: &str) -> Vec<(PathBuf, &'static [&'static str])> {
    let pf = env.program_files();
    let mut v: Vec<(PathBuf, &'static [&'static str])> = Vec::new();
    if tool == "java" {
        v.push((
            PathBuf::from("/Library/Java/JavaVirtualMachines"),
            &["Contents", "Home"],
        ));
        v.push((PathBuf::from("/usr/lib/jvm"), &[]));
        v.push((PathBuf::from("/usr/lib64/jvm"), &[]));
        v.push((PathBuf::from("/usr/java"), &[]));
        for p in &pf {
            for vendor in [
                "Java",
                "Eclipse Adoptium",
                "Microsoft",
                "Zulu",
                "Amazon Corretto",
                "BellSoft",
                "Semeru",
                "OpenJDK",
            ] {
                v.push((p.join(vendor), &[]));
            }
        }
    }
    v
}

/// Single, fixed runtime homes (no version children).
pub fn fixed_homes(env: &Env, tool: &str) -> Vec<PathBuf> {
    let pf = env.program_files();
    let mut v = Vec::new();
    match tool {
        "go" => {
            v.push(PathBuf::from("/usr/local/go"));
            v.push(PathBuf::from("/usr/lib/go"));
            v.extend(pf.iter().map(|p| p.join("Go")));
            if cfg!(windows) {
                v.push(PathBuf::from(r"C:\Go"));
            }
        }
        "node" => {
            v.extend(pf.iter().map(|p| p.join("nodejs")));
        }
        "python" => {
            for p in &pf {
                for mm in ["313", "312", "311", "310", "39", "38"] {
                    v.push(p.join(format!("Python{mm}")));
                }
            }
        }
        _ => {}
    }
    v
}

// ------------------------------------------------------------------ ownership

/// Who owns a runtime home, and the details needed to uninstall it through that owner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Owner {
    /// `homebrew`, `nvm`, `nvm-windows`, `pyenv`, `pyenv-win`, `sdkman`, `asdf`, `fnm`,
    /// `mise`, `scoop`, `scoop-global`, `winget`, `chocolatey`, `installer` (Windows
    /// vendor installer / MSI), `system` or `manual`.
    pub provider: String,
    /// Package in the owner's terms: Homebrew formula, scoop app, winget id, chocolatey
    /// package, sdkman candidate, asdf / mise plugin.
    pub package: Option<String>,
    /// The owner's data dir (NVM_DIR, PYENV_ROOT, SDKMAN_DIR, FNM_DIR, scoop root, ...).
    pub store: Option<PathBuf>,
    /// Version id as the owner names it (directory name in its store).
    pub id: Option<String>,
}

impl Owner {
    fn new(p: &str) -> Self {
        Owner {
            provider: p.into(),
            ..Default::default()
        }
    }
}

/// Segments of `key` after `root_key/` (both from [`path_key_for`]).
fn rel_segments<'a>(key: &'a str, root_key: &str) -> Option<Vec<&'a str>> {
    let rest = key.strip_prefix(root_key)?;
    let rest = if root_key.ends_with('/') {
        rest
    } else {
        rest.strip_prefix('/')?
    };
    Some(rest.split('/').filter(|s| !s.is_empty()).collect())
}

/// Classify a runtime home. Matching is separator-agnostic and, for Windows-only
/// layouts, case-insensitive, so it gives the same answer on every host OS.
pub fn owner(home: &Path, env: &Env) -> Owner {
    let raw = path_key_for(home, false);
    let low = raw.to_ascii_lowercase();
    // original-case segments, for ids / packages
    let raw_segs: Vec<&str> = raw.split('/').collect();
    let seg_at = |low_idx: usize| -> Option<String> {
        // index into the segments of `low` == index into `raw` (same structure)
        raw_segs.get(low_idx).map(|s| s.to_string())
    };
    let low_segs: Vec<&str> = low.split('/').collect();

    // Homebrew (any Cellar)
    if let Some(i) = raw_segs.iter().position(|c| *c == "Cellar") {
        if let Some(f) = raw_segs.get(i + 1).filter(|f| !f.is_empty()) {
            return Owner {
                provider: "homebrew".into(),
                package: Some(f.to_string()),
                store: Some(PathBuf::from(raw_segs[..=i].join("/"))),
                id: raw_segs.get(i + 2).map(|s| s.to_string()),
            };
        }
    }

    // Stores rooted at a known (possibly overridden) manager dir.
    // (provider, sub path under the manager dir, segments: [package?, id])
    let layouts: &[(&str, &[&str], bool)] = &[
        ("nvm", &["versions", "node"], false),
        ("pyenv-win", &["versions"], false),
        ("pyenv", &["versions"], false),
        ("sdkman", &["candidates"], true),
        ("asdf", &["installs"], true),
        ("mise", &["installs"], true),
        ("fnm", &["node-versions"], false),
        ("nvm-windows", &[], false),
        ("scoop", &["apps"], true),
        ("scoop-global", &["apps"], true),
    ];
    for (prov, d) in env.manager_dirs() {
        let Some((_, sub, has_pkg)) = layouts.iter().find(|(p, _, _)| *p == prov) else {
            continue;
        };
        let mut root = d.clone();
        for s in sub.iter() {
            root = root.join(s);
        }
        let fold = cfg!(windows) || prov.contains("windows") || prov.starts_with("scoop");
        let key = path_key_for(home, fold);
        let Some(segs) = rel_segments(&key, &path_key_for(&root, fold)) else {
            continue;
        };
        let orig = rel_segments(&raw, &path_key_for(&root, false)).unwrap_or(segs.clone());
        let (pkg, id) = if *has_pkg {
            (orig.first().map(|s| s.to_string()), orig.get(1))
        } else {
            (None, orig.first())
        };
        let Some(id) = id else { continue };
        if prov == "nvm-windows" && !id.starts_with('v') {
            continue;
        }
        return Owner {
            provider: prov.into(),
            package: pkg,
            store: Some(d),
            id: Some(id.to_string()),
        };
    }

    // Default layouts by marker (data dirs under a different home, e.g. other users).
    let markers: &[(&str, &str, usize, bool)] = &[
        // (marker in lower-case key, provider, chars of marker belonging to the store, pkg)
        ("/.nvm/versions/node/", "nvm", 5, false),
        ("/.pyenv/pyenv-win/versions/", "pyenv-win", 17, false),
        ("/.pyenv/versions/", "pyenv", 7, false),
        ("/.sdkman/candidates/", "sdkman", 8, true),
        ("/.asdf/installs/", "asdf", 6, true),
        ("/mise/installs/", "mise", 5, true),
        ("/fnm/node-versions/", "fnm", 4, false),
        ("/appdata/roaming/nvm/", "nvm-windows", 20, false),
        ("/programdata/scoop/apps/", "scoop-global", 18, true),
        ("/scoop/apps/", "scoop", 6, true),
    ];
    for (marker, prov, store_len, has_pkg) in markers {
        let Some(i) = low.find(marker) else { continue };
        let rest: Vec<&str> = raw[i + marker.len()..]
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        let (pkg, id) = if *has_pkg {
            (rest.first().map(|s| s.to_string()), rest.get(1))
        } else {
            (None, rest.first())
        };
        let Some(id) = id else { continue };
        if *prov == "nvm-windows" && !id.starts_with('v') {
            continue;
        }
        return Owner {
            provider: prov.to_string(),
            package: pkg,
            store: Some(PathBuf::from(&raw[..i + store_len])),
            id: Some(id.to_string()),
        };
    }

    // winget portable packages: ...\Microsoft\WinGet\Packages\<Id>_<Source>
    if let Some(i) = low_segs
        .windows(3)
        .position(|w| w == ["microsoft", "winget", "packages"])
    {
        if let Some(dir) = seg_at(i + 3) {
            let id = dir.split('_').next().unwrap_or(&dir).to_string();
            return Owner {
                provider: "winget".into(),
                package: Some(id),
                ..Default::default()
            };
        }
    }
    // chocolatey: ...\chocolatey\lib\<pkg>\...
    if let Some(i) = low_segs.windows(2).position(|w| w == ["chocolatey", "lib"]) {
        if let Some(pkg) = seg_at(i + 2) {
            return Owner {
                provider: "chocolatey".into(),
                package: Some(pkg),
                store: Some(PathBuf::from(raw_segs[..=i].join("/"))),
                ..Default::default()
            };
        }
    }

    if is_system_path(&raw) {
        return Owner::new("system");
    }
    if is_windows_installer_path(&low, env) {
        return Owner::new("installer");
    }
    Owner::new("manual")
}

/// OS-owned locations: macOS system / Xcode, Linux distro packages (/usr outside
/// /usr/local, /nix/store, /snap), Windows system dirs and Store app stubs.
fn is_system_path(raw: &str) -> bool {
    let low = raw.to_ascii_lowercase();
    let s = raw;
    s == "/usr"
        || (s.starts_with("/usr/") && !s.starts_with("/usr/local/"))
        || s.starts_with("/System/")
        || s.starts_with("/Library/Developer/CommandLineTools")
        || (s.starts_with("/Applications/Xcode") && s.contains(".app/"))
        || s.starts_with("/nix/store/")
        || s.starts_with("/snap/")
        || s == "/bin"
        || s.starts_with("/bin/")
        || s.starts_with("/opt/local/") // MacPorts
        || is_drive_path(&low, "windows")
        || low.contains("/microsoft/windowsapps")
        || low.contains("/program files/windowsapps/")
}

/// Path after `x:/` for a drive-letter path.
fn drive_rest(low: &str) -> Option<&str> {
    let b = low.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && &low[1..3] == ":/").then(|| &low[3..])
}

/// `x:/<rest>` or `x:/<rest>/...` for any drive letter.
fn is_drive_path(low: &str, rest: &str) -> bool {
    let b = low.as_bytes();
    b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && &low[1..3] == ":/"
        && (low[3..] == *rest || low[3..].starts_with(&format!("{rest}/")))
}

/// Installed by a Windows installer (MSI / vendor setup / python.org): Program Files,
/// `%LOCALAPPDATA%\Programs`, `C:\PythonXY`, `C:\Go`. Removing these by deleting the
/// directory would leave registry entries behind, so runa refuses to purge them.
fn is_windows_installer_path(low: &str, env: &Env) -> bool {
    let under = |root: &Path| {
        let r = path_key_for(root, true);
        low.starts_with(&format!("{r}/"))
    };
    is_drive_path(low, "program files")
        || is_drive_path(low, "program files (x86)")
        || env.program_files.iter().any(|p| under(p))
        || env
            .local_appdata
            .as_ref()
            .is_some_and(|l| under(&l.join("Programs")))
        || low.contains("/appdata/local/programs/")
        || drive_rest(low).is_some_and(|r| {
            let first = r.split('/').next().unwrap_or("");
            first == "go" || first.starts_with("python")
        })
}

/// Shared prefixes that are never a runtime root by themselves: `/`, `/usr`,
/// `/usr/local`, Homebrew / Linuxbrew / MacPorts prefixes, `/opt`, `/home`, drive roots,
/// `C:\Windows`, `C:\Program Files`, `C:\ProgramData`, `C:\Users`, the user's home and
/// the Windows app-data roots.
pub fn is_system_prefix(p: &Path, env: &Env) -> bool {
    let low = path_key_for(p, true);
    const UNIX: &[&str] = &[
        "/",
        "/usr",
        "/usr/local",
        "/usr/lib",
        "/usr/share",
        "/opt",
        "/opt/homebrew",
        "/opt/local",
        "/home",
        "/home/linuxbrew",
        "/home/linuxbrew/.linuxbrew",
        "/snap",
        "/nix",
        "/library",
        "/system",
        "/users",
        "/var",
        "/etc",
        "/bin",
        "/sbin",
    ];
    if UNIX.contains(&low.as_str()) {
        return true;
    }
    let b = low.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        let rest = low[2..].trim_start_matches('/');
        if matches!(
            rest,
            "" | "windows"
                | "windows/system32"
                | "windows/syswow64"
                | "program files"
                | "program files (x86)"
                | "programdata"
                | "users"
        ) {
            return true;
        }
    }
    let same = |q: &Path| path_key_for(q, true) == low;
    same(&env.home)
        || same(&env.home.join(".linuxbrew"))
        || env.appdata.as_deref().is_some_and(same)
        || env
            .local_appdata
            .as_deref()
            .is_some_and(|l| same(l) || same(&l.join("Programs")) || same(&l.join("Microsoft")))
        || env.program_files.iter().any(|q| same(q))
        || env.program_data.as_deref().is_some_and(same)
}

/// A version-manager / package-manager shim dir (`~/.pyenv/shims`, `~/scoop/shims`,
/// `pyenv-win\shims`, `~/.asdf/shims`, mise shims, ...): never a runtime home.
pub fn is_shim_dir(p: &Path) -> bool {
    let k = path_key_for(p, true);
    k.ends_with("/shims") || k.contains("/shims/")
}

/// Parse `py -0p` (Windows py launcher) output into interpreter paths.
///
/// ```text
///  -V:3.12 *        C:\Users\a\AppData\Local\Programs\Python\Python312\python.exe
///  -3.9-64          C:\Python39\python.exe
/// ```
pub fn parse_py_launcher(out: &str) -> Vec<PathBuf> {
    out.lines()
        .filter_map(|l| {
            let b = l.as_bytes();
            let i = (1..b.len().saturating_sub(1))
                .find(|&i| b[i] == b':' && b[i + 1] == b'\\' && b[i - 1].is_ascii_alphabetic())?;
            let p = l[i - 1..].trim();
            p.to_ascii_lowercase()
                .ends_with(".exe")
                .then(|| PathBuf::from(p))
        })
        .collect()
}

/// A Linux distro package that owns `path`, via dpkg or rpm (`None` when neither knows).
pub fn linux_package_owner(path: &Path) -> Option<(String, String)> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = std::process::Command::new(cmd)
            .args(args)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).to_string())
    };
    if let Some(o) = run("dpkg-query", &["-S"]) {
        // "openjdk-17-jre-headless:amd64: /usr/lib/jvm/..."
        let pkg = o
            .lines()
            .next()?
            .split(": ")
            .next()?
            .split(',')
            .next()?
            .trim();
        let pkg = pkg.split(':').next().unwrap_or(pkg).to_string();
        if !pkg.is_empty() {
            return Some(("apt".into(), pkg));
        }
    }
    if let Some(o) = run("pacman", &["-Qoq"]) {
        let pkg = o.lines().next()?.trim().to_string();
        if !pkg.is_empty() && !pkg.contains(' ') {
            return Some(("pacman".into(), pkg));
        }
    }
    if let Some(o) = run("rpm", &["-qf", "--qf", "%{NAME}\\n"]) {
        let pkg = o.lines().next()?.trim().to_string();
        if !pkg.is_empty() && !pkg.contains(' ') {
            return Some(("dnf".into(), pkg));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_at(home: &Path) -> Env {
        Env {
            home: home.to_path_buf(),
            ..Default::default()
        }
    }

    fn win_env() -> Env {
        Env {
            home: PathBuf::from(r"C:\Users\a"),
            appdata: Some(PathBuf::from(r"C:\Users\a\AppData\Roaming")),
            local_appdata: Some(PathBuf::from(r"C:\Users\a\AppData\Local")),
            program_files: vec![
                PathBuf::from(r"C:\Program Files"),
                PathBuf::from(r"C:\Program Files (x86)"),
            ],
            program_data: Some(PathBuf::from(r"C:\ProgramData")),
            ..Default::default()
        }
    }

    fn own(p: &str, env: &Env) -> Owner {
        owner(Path::new(p), env)
    }

    #[test]
    fn windows_layouts_classify_on_any_host() {
        let e = win_env();
        let o = own(r"C:\Users\a\AppData\Roaming\nvm\v20.11.1", &e);
        assert_eq!(o.provider, "nvm-windows");
        assert_eq!(o.id.as_deref(), Some("v20.11.1"));
        // other user / no env: marker based, case-insensitive
        assert_eq!(
            own(r"D:\USERS\b\APPDATA\ROAMING\NVM\v18.0.0", &Env::default()).provider,
            "nvm-windows"
        );
        let o = own(
            r"C:\Users\a\AppData\Roaming\fnm\node-versions\v22.1.0\installation",
            &e,
        );
        assert_eq!(
            (o.provider.as_str(), o.id.as_deref()),
            ("fnm", Some("v22.1.0"))
        );
        let o = own(r"C:\Users\a\.pyenv\pyenv-win\versions\3.12.1", &e);
        assert_eq!(o.provider, "pyenv-win");
        assert_eq!(o.id.as_deref(), Some("3.12.1"));
        assert_eq!(
            path_key_for(o.store.as_deref().unwrap(), true),
            "c:/users/a/.pyenv/pyenv-win"
        );
        let o = own(r"C:\Users\a\scoop\apps\nodejs-lts\20.11.1", &e);
        assert_eq!(o.provider, "scoop");
        assert_eq!(o.package.as_deref(), Some("nodejs-lts"));
        assert_eq!(o.id.as_deref(), Some("20.11.1"));
        assert_eq!(
            own(r"C:\ProgramData\scoop\apps\python\3.12.1", &e).provider,
            "scoop-global"
        );
        let o = own(
            r"C:\Users\a\AppData\Local\Microsoft\WinGet\Packages\GoLang.Go_Microsoft.Winget.Source_8wekyb3d8bbwe\go",
            &e,
        );
        assert_eq!(o.provider, "winget");
        assert_eq!(o.package.as_deref(), Some("GoLang.Go"));
        let o = own(r"C:\ProgramData\chocolatey\lib\golang\tools\go", &e);
        assert_eq!(o.provider, "chocolatey");
        assert_eq!(o.package.as_deref(), Some("golang"));
        for p in [
            r"C:\Program Files\Eclipse Adoptium\jdk-21.0.2.13-hotspot",
            r"C:\Program Files\Microsoft\jdk-17.0.10.7-hotspot",
            r"C:\Program Files\Go",
            r"C:\Program Files\nodejs",
            r"C:\Users\a\AppData\Local\Programs\Python\Python312",
            r"C:\Python311",
            r"C:\Go",
        ] {
            assert_eq!(own(p, &e).provider, "installer", "{p}");
        }
        for p in [
            r"C:\Windows\py",
            r"C:\Users\a\AppData\Local\Microsoft\WindowsApps",
            r"C:\Program Files\WindowsApps\PythonSoftwareFoundation.Python.3.12_3.12.1_x64",
        ] {
            assert_eq!(own(p, &e).provider, "system", "{p}");
        }
        assert_eq!(own(r"D:\tools\jdk-21", &e).provider, "manual");
    }

    #[test]
    fn linux_layouts() {
        let e = env_at(Path::new("/home/u"));
        assert_eq!(
            own("/home/linuxbrew/.linuxbrew/Cellar/node/22.1.0", &e).package,
            Some("node".into())
        );
        assert_eq!(
            own("/home/u/.linuxbrew/Cellar/go/1.22.0/libexec", &e).provider,
            "homebrew"
        );
        assert_eq!(
            own("/usr/lib/jvm/java-17-openjdk-amd64", &e).provider,
            "system"
        );
        assert_eq!(own("/usr/lib/go-1.22", &e).provider, "system");
        assert_eq!(own("/nix/store/abc-nodejs-20", &e).provider, "system");
        assert_eq!(own("/usr/local/go", &e).provider, "manual");
        assert_eq!(own("/opt/jdk-21", &e).provider, "manual");
        let o = own(
            "/home/u/.local/share/fnm/node-versions/v20.1.0/installation",
            &e,
        );
        assert_eq!(
            (o.provider.as_str(), o.id.as_deref()),
            ("fnm", Some("v20.1.0"))
        );
        assert_eq!(o.store, Some(PathBuf::from("/home/u/.local/share/fnm")));
        let o = own("/home/u/.local/share/mise/installs/python/3.12.1", &e);
        assert_eq!(o.provider, "mise");
        assert_eq!(o.package.as_deref(), Some("python"));
        assert_eq!(o.id.as_deref(), Some("3.12.1"));
    }

    #[test]
    fn env_overrides_win_over_defaults() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let mut e = env_at(h);
        let custom = h.join("tools").join("nvm");
        e.vars.push(("NVM_DIR".into(), custom.clone()));
        e.vars.push(("PYENV_ROOT".into(), h.join("py")));
        let o = owner(&custom.join("versions").join("node").join("v20.1.0"), &e);
        assert_eq!(o.provider, "nvm");
        assert_eq!(o.store, Some(custom));
        assert_eq!(o.id.as_deref(), Some("v20.1.0"));
        let o = owner(&h.join("py").join("versions").join("3.11.2"), &e);
        assert_eq!(o.provider, "pyenv");
        assert_eq!(o.id.as_deref(), Some("3.11.2"));
    }

    #[test]
    fn system_prefixes_cover_all_platforms() {
        let e = win_env();
        for p in [
            "/",
            "/usr",
            "/usr/local",
            "/opt/homebrew",
            "/home/linuxbrew/.linuxbrew",
            "C:\\",
            r"C:\Windows",
            r"c:\windows\System32",
            r"C:\Program Files",
            r"C:\Program Files (x86)",
            r"C:\ProgramData",
            r"C:\Users\a",
            r"C:\Users\a\AppData\Local\Microsoft",
        ] {
            assert!(is_system_prefix(Path::new(p), &e), "{p}");
        }
        for p in [
            "/usr/local/go",
            r"C:\Program Files\Go",
            "/usr/lib/jvm/java-17",
        ] {
            assert!(!is_system_prefix(Path::new(p), &e), "{p}");
        }
    }

    #[test]
    fn store_dirs_follow_platform_layouts() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let mut e = env_at(h);
        e.appdata = Some(h.join("AppData").join("Roaming"));
        e.local_appdata = Some(h.join("AppData").join("Local"));
        let dirs = |tool: &str| -> Vec<(PathBuf, &'static str)> {
            stores(&e, tool)
                .into_iter()
                .map(|s| (s.dir, s.label))
                .collect()
        };
        let node = dirs("node");
        assert!(node.contains(&(h.join("AppData").join("Roaming").join("nvm"), "nvm-windows")));
        assert!(node.contains(&(
            h.join("AppData")
                .join("Roaming")
                .join("fnm")
                .join("node-versions"),
            "fnm"
        )));
        assert!(node.contains(&(
            h.join(".local")
                .join("share")
                .join("fnm")
                .join("node-versions"),
            "fnm"
        )));
        assert!(node.contains(&(h.join("scoop").join("apps").join("nodejs"), "scoop")));
        let py = dirs("python");
        assert!(py.contains(&(
            h.join(".pyenv").join("pyenv-win").join("versions"),
            "pyenv-win"
        )));
        assert!(py.contains(&(
            h.join("AppData")
                .join("Local")
                .join("Programs")
                .join("Python"),
            WELL_KNOWN
        )));
        let go = dirs("go");
        assert!(go.contains(&(
            h.join(".local")
                .join("share")
                .join("mise")
                .join("installs")
                .join("go"),
            "mise"
        )));
        e.vars.push(("FNM_DIR".into(), h.join("f")));
        assert!(stores(&e, "node")
            .iter()
            .any(|s| s.dir == h.join("f").join("node-versions")));
        assert!(!stores(&e, "node").iter().any(|s| s.dir
            == h.join(".local")
                .join("share")
                .join("fnm")
                .join("node-versions")));
    }

    #[test]
    fn shim_dirs() {
        assert!(is_shim_dir(Path::new(r"C:\Users\a\scoop\shims")));
        assert!(is_shim_dir(&Path::new("/h").join(".pyenv").join("shims")));
        assert!(!is_shim_dir(Path::new("/h/.pyenv/versions/3.12.1")));
    }

    #[test]
    fn py_launcher_output() {
        let out = " -V:3.12 *        C:\\Users\\a\\AppData\\Local\\Programs\\Python\\Python312\\python.exe\n -3.9-64          C:\\Python39\\python.exe\n -V:ContinuumAnalytics/Anaconda39-64 C:\\Users\\a\\anaconda3\\python.exe\n\nNo extra\n";
        let v = parse_py_launcher(out);
        assert_eq!(v.len(), 3);
        assert_eq!(v[1], PathBuf::from("C:\\Python39\\python.exe"));
    }
}
