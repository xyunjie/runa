<div align="center">

<img src="assets/logo/logo.svg" width="96" height="96" alt="Runa logo">

# Runa

**One cross-platform runtime version manager for Node.js, Java, Python, Go, Rust, Ruby, Bun and Deno — CLI, desktop GUI and MCP server.**

English · [简体中文](README.zh-CN.md)

[![CI](https://github.com/xyunjie/runa/actions/workflows/ci.yml/badge.svg)](https://github.com/xyunjie/runa/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-informational)
![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange?logo=rust)
![Tauri v2](https://img.shields.io/badge/GUI-Tauri%20v2-24C8DB?logo=tauri)

</div>

Runa is a **runtime version manager** written in Rust that runs natively on **macOS, Windows and Linux**. It installs and switches versions of Node.js, Java (JDK), Python, Go, Rust, Ruby, Bun and Deno from one tool, reads the version files your projects already have (`.tool-versions`, `.nvmrc`, `.python-version`, `.java-version`, `go.mod`, `pom.xml`, …) and switches automatically per project. Use it as a **Node.js version manager / nvm alternative**, a **pyenv alternative**, an **SDKMAN! alternative** for Java, or a single-binary **asdf / mise alternative** — with a Tauri desktop app and a built-in **MCP server** so AI coding agents can see and fix your toolchain.

## Table of contents

- [Highlights](#highlights)
- [Screenshots](#screenshots)
- [Install](#install)
- [Quick start](#quick-start)
- [Supported runtimes and version managers](#supported-runtimes-and-version-managers)
- [Per-project automatic version switching](#per-project-automatic-version-switching)
- [Desktop GUI](#desktop-gui)
- [MCP server for AI agents](#mcp-server-for-ai-agents)
- [Comparison with nvm, pyenv, SDKMAN!, asdf and mise](#comparison-with-nvm-pyenv-sdkman-asdf-and-mise)
- [FAQ](#faq)
- [Contributing](#contributing)
- [License](#license)

## Highlights

- **One tool, eight runtimes** — Node.js, Java, Python, Go, Rust, Ruby, Bun, Deno, plus more through [JSON plugins](examples/plugins/sample.json).
- **Cross-platform** — macOS, Windows and Linux; shell hooks for bash, zsh, fish and PowerShell, with shims as a fallback for IDEs and scripts.
- **Reads existing version files** — `.tool-versions` (asdf-compatible), `.nvmrc`, `.node-version`, `package.json` `engines.node`, `.python-version`, `.java-version`, `.sdkmanrc`, `go.mod`, `rust-toolchain.toml`, and Java versions from Maven `pom.xml` / Gradle builds.
- **Fuzzy version specs** — `20`, `3.12`, `latest`, `lts`, `^18.2`, `>=18 <21`, `18.x`, `temurin-21`, `go1.22`.
- **Isolated installs** — every version lives in `~/.runa/installs/<tool>/<version>`; switching only moves a pointer.
- **Adopt what you already have** — detects runtimes installed by nvm, fnm, pyenv, SDKMAN!, asdf, Homebrew and system locations and manages them in place, without moving or linking anything.
- **Safe downloads** — SHA256 verification whenever upstream publishes checksums, zip-slip protection, mirrors (e.g. npmmirror, golang.google.cn) and HTTP/SOCKS5 proxy support.
- **Desktop app** — a Tauri v2 GUI for overview, installs, projects and settings, in light and dark themes.
- **MCP server** — `runa mcp` lets Cursor, Claude, Codex, VS Code, Windsurf and OpenCode inspect and (with your permission) change runtime versions.

## Screenshots

| Light | Dark |
|---|---|
| ![Runtime detail, light](docs/screens/tool-installed-light.png) | ![Runtime detail, dark](docs/screens/tool-installed-dark.png) |
| ![Installing, light](docs/screens/installing-light.png) | ![Settings, dark](docs/screens/settings-dark.png) |

<!-- TODO: add screenshots of the Projects (项目) and MCP pages, e.g. docs/screens/projects-light.png and docs/screens/mcp-light.png -->

## Install

> Prebuilt binaries are not published yet. The [release workflow](.github/workflows/release.yml) builds the CLI for Linux (x86_64 / arm64), macOS (Intel / Apple Silicon) and Windows (x86_64) plus GUI installers; once a release is tagged they will appear on the [Releases](https://github.com/xyunjie/runa/releases) page and `runa update` can keep the CLI current.

Build from source (Rust 1.88+):

```bash
cargo install --git https://github.com/xyunjie/runa runa-cli   # installs the `runa` binary
runa setup                                                     # add the activation hook to your shell config
```

Or clone and build:

```bash
git clone https://github.com/xyunjie/runa && cd runa
cargo build --release -p runa-cli        # target/release/runa
```

Prefer not to edit your shell config? Activate manually:

```bash
eval "$(runa activate zsh)"                                    # bash: runa activate bash
runa activate fish | source                                    # fish
(& runa activate powershell | Out-String) | Invoke-Expression  # PowerShell
```

## Quick start

```bash
runa install node 20          # newest 20.x; the first installed version becomes the global default
runa install java 21
runa install python 3.12
runa use node 22              # pin for this project (writes the project's version file)
runa use -g go latest         # set the global version (installs it if missing)
runa install                  # install everything the current project needs
runa current                  # what is active here, and which file decided it
runa list                     # installed versions
runa list-all node 22         # remote versions
runa exec --with node@20 -- node -v
runa detect && runa adopt     # find and adopt runtimes from nvm / pyenv / SDKMAN! / Homebrew …
runa doctor
```

Run `runa help <command>` for details on any command; the [Chinese README](README.zh-CN.md#命令一览) has the full command reference.

## Supported runtimes and version managers

| Runtime | Installed by Runa from | Checksum |
|---|---|---|
| Node.js | nodejs.org official builds | `SHASUMS256.txt` |
| Java (JDK) | Eclipse Temurin (Adoptium API) | SHA256 from the API |
| Python | astral-sh/python-build-standalone (no compiling) | `SHA256SUMS` |
| Go | go.dev/dl | SHA256 in the JSON index |
| Bun | GitHub official releases | `SHASUMS256.txt` |
| Deno | GitHub official releases | `.sha256sum` per asset (2.1+) |
| Rust | via rustup / mise / asdf | — |
| Ruby | via rbenv / mise (upstream ships source only) | — |

`runa manager` can show, install, upgrade and uninstall these version managers: **nvm, fnm, pyenv, SDKMAN!, rustup, rbenv, asdf, mise** (and their Windows counterparts). `runa installer` lets you choose whether new versions are installed by Runa itself or by a manager you already use.

## Per-project automatic version switching

Runa resolves a version for each tool in this order:

1. Environment variable `RUNA_<TOOL>_VERSION` (e.g. `RUNA_NODE_VERSION=20`).
2. Walking up from the current directory: `.tool-versions`, then the tool's own files — `.nvmrc`, `.node-version`, `package.json` `engines.node`, `.python-version`, `runtime.txt`, `.java-version`, `.sdkmanrc`, `.go-version`, `go.mod`, `rust-toolchain(.toml)`, `.ruby-version`, `.bun-version`, `.dvmrc`.
3. Build files, when no explicit version file exists: Java from Maven `pom.xml` (compiler `release`/`source`/`target`, `java.version`, parent POMs) and Gradle (`JavaLanguageVersion.of`, `jvmToolchain`, `sourceCompatibility`); Python from `pyproject.toml` `requires-python` / Poetry, `Pipfile`, `setup.py` `python_requires` or a project `.venv`.
4. The global `~/.runa/.tool-versions`.

Ranges such as `>=18 <21` or `>=3.10,<3.13` pick the newest installed version inside the range. `runa project init` additionally gives a project its own isolated environment (`local`), shared versions (`versions`) or a Docker container (`docker`).

## Desktop GUI

The desktop app (Tauri v2 + React) shares `runa-core` and the data directory with the CLI:

- **Overview** of every runtime's active version, source and disk usage, with one-click shell setup.
- **Runtime pages** to browse installed and remote versions (search, LTS filter), install with progress, set global, uninstall.
- **Projects** — add folders (or a parent folder with many sub-projects), see what each project requires, whether it is satisfied, and fill gaps in one click.
- **Detect & adopt**, **MCP** client setup, and **Settings** for mirrors, proxy, shell activation and theme.

Build it with Node.js 20.19+ and Rust 1.90+:

```bash
cd gui/web && npm ci && npm run build && cd ../..
cargo build --release -p runa-gui
```

See the [Chinese README](README.zh-CN.md#桌面端) for Linux packages, dev mode and installer bundling.

## MCP server for AI agents

`runa mcp` is a stdio [Model Context Protocol](https://modelcontextprotocol.io) server:

- Read-only tools: `list_runtimes`, `list_available`, `detect_project`, `list_managers`.
- Write tools: `install`, `set_global`, `uninstall` — governed by a policy set in the desktop app (read-only by default; destructive actions require confirmation in the GUI).
- One-click setup in the desktop app's MCP page for **Cursor, VS Code, Claude Desktop, Claude Code, Codex, Windsurf and OpenCode**.

There is also a project-bound server, `runa mcp serve --project <dir> [--read-only]`, and `runa mcp config --project . --client cursor` prints a client config without modifying it. `runa context --json` gives agents a stable read-only summary of what a project needs.

## Comparison with nvm, pyenv, SDKMAN!, asdf and mise

| | Runa | nvm | pyenv | SDKMAN! | asdf | mise |
|---|---|---|---|---|---|---|
| Languages | 8 built in + JSON plugins | Node.js | Python | JVM ecosystem | Many (plugins) | Many |
| Native Windows | Yes | No (nvm-windows is separate) | No (pyenv-win is separate) | No (WSL / Git Bash) | No | Yes |
| Desktop GUI | Yes | No | No | No | No | No |
| MCP server | Yes | No | No | No | No | No |
| Reads `.tool-versions` | Yes | No | No | No | Yes | Yes |
| Adopts existing installs | Yes | — | — | — | — | — |
| Maturity | New (0.1) | Very mature | Very mature | Very mature | Mature | Mature |

Runa is young: asdf and mise have far larger plugin ecosystems, and mise also manages env vars and tasks. Runa's focus is a batteries-included experience for the most common runtimes, first-class Windows support, a GUI, and safe AI-agent integration. You can keep using your existing managers — Runa detects and adopts their versions instead of replacing them.

## FAQ

**Does Runa conflict with nvm / pyenv / SDKMAN!?**
No. `runa detect` and `runa adopt` register their installs in place; nothing is moved or relinked. `runa uninstall --forget` only removes Runa's record.

**Where is everything stored?**
In `~/.runa` (override with `RUNA_HOME`): installs, shims, settings, plugins and caches. Delete it to remove Runa completely.

**Do I need the shell hook?**
It is recommended (`runa setup`). Without it, the shims in `~/.runa/shims` still pick the right version for IDEs and scripts.

**How do I use a mirror or proxy?**
`runa mirror cn` and `runa proxy socks5h://127.0.0.1:1080` (or `system` / `off`).

**Can AI agents break my setup?**
The global MCP server is read-only by default; write access and confirmation rules are configured in the desktop app.

## Contributing

Issues and pull requests are welcome.

```bash
cargo fmt --all && cargo clippy --workspace --all-targets
cargo test --workspace
cd gui/web && npm run build     # the frontend also runs in a browser with mock data: npm run dev
```

Project layout: `crates/runa-core` (core library), `crates/runa-cli` (`runa` binary), `gui/src-tauri` + `gui/web` (desktop app), `examples/plugins` (JSON plugin samples). Design inspired in part by [hquip/Vaelen](https://github.com/hquip/Vaelen).

## License

[MIT](LICENSE)
