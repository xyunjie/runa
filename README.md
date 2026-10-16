# Runa

**Runa** 是一个跨平台（Windows / macOS / Linux）的多语言运行时版本管理器，用 Rust 编写，提供命令行工具 `runa` 和基于 Tauri v2 的桌面客户端。一个工具统一管理 Node.js、Java、Python、Go 等开发版本，并兼容 asdf 的 `.tool-versions`。

功能设计参考了 [hquip/Vaelen](https://github.com/hquip/Vaelen)（Go + Wails 实现的 `vsm`）。

![概览](docs/screens/overview-light.png)

## 特性

- **隔离安装**：每个版本装在 `~/.runa/installs/<工具>/<版本>`，切换版本只改指针（`.tool-versions`），不复制、不覆盖。
- **自动解析版本**，优先级从高到低：
  1. 环境变量 `RUNA_<TOOL>_VERSION`（如 `RUNA_NODE_VERSION=20`）
  2. 从当前目录向上查找，最近的目录优先：`.tool-versions`，以及各语言的常用文件
     `.nvmrc` / `.node-version` / `package.json` 的 `engines.node`、`.python-version`、`.java-version` / `.sdkmanrc`、`.go-version` / `go.mod`（`toolchain` 优先于 `go` 指令）
  3. 全局 `~/.runa/.tool-versions`
- **模糊版本**：`20`、`3.12`、`latest`、`lts`、`^18.2`、`>=18`、`18.x`、`temurin-21`、`go1.22` 等。
- **两种激活方式**：shell 钩子（bash / zsh / fish / PowerShell）在每次提示符时重建 PATH；shims 作为兜底（IDE、脚本、非交互 shell）。
- **内置插件**：
  | 工具 | 来源 | 校验 |
  |---|---|---|
  | node | nodejs.org/dist 官方预编译包 | `SHASUMS256.txt` |
  | java | Eclipse Temurin（Adoptium API） | API 提供的 SHA256 |
  | python | astral-sh/python-build-standalone（`install_only`，免编译） | `SHA256SUMS` |
  | go | go.dev/dl | JSON 元数据中的 SHA256 |
- **JSON 声明式插件**：在 `~/.runa/plugins/*.json` 描述下载地址即可管理 deno、bun 等更多工具，示例见 [`examples/plugins/sample.json`](examples/plugins/sample.json)。
- **检测与接管**：扫描 PATH、`JAVA_HOME`、`GOROOT` 及常见目录中的已有安装，接管时只建符号链接，原安装不受影响。
- **镜像与代理**：一键切换国内镜像（npmmirror、golang.google.cn），代理支持 `http` / `https` / `socks5` / `socks5h`、`off`、`system`。
- **安全**：解压时防 zip-slip（拒绝 `..`、绝对路径以及指向目录外的符号链接）；上游提供校验值时强制 SHA256 校验。

## 快速上手

```bash
runa install node 20          # 安装 20.x 的最新版（首次安装的版本自动设为全局默认）
runa install java 21
runa install python 3.12
runa use node 22              # 写入当前目录的 .tool-versions
runa use -g go latest         # 设为全局版本（未安装会自动安装）
runa current                  # 查看当前目录生效的版本及来源
runa setup                    # 一键把激活脚本写入 shell 配置文件，重开终端生效
```

不想改 shell 配置时，也可以手动启用：

```bash
# bash / zsh
eval "$(runa activate bash)"     # zsh 用 runa activate zsh
# fish
runa activate fish | source
# PowerShell
(& runa activate powershell | Out-String) | Invoke-Expression
```

## 命令一览

| 命令 | 说明 |
|---|---|
| `runa install [工具] [版本] [--require-checksum]` | 安装；支持 `node@20` 写法；不带参数时安装当前目录需要但缺失的全部版本；`--require-checksum`（或 `RUNA_REQUIRE_CHECKSUM=1`）在上游没有 SHA256 时拒绝安装 |
| `runa uninstall <工具> <版本>` | 卸载（对接管的版本只删除链接） |
| `runa use <工具> <版本> [-g]` | 设置项目版本 / 全局版本，未安装时自动安装（`--no-install` 关闭） |
| `runa list [工具] [--json]` | 已安装版本，`*` 为当前生效 |
| `runa list-all <工具> [前缀]` | 可安装的远程版本 |
| `runa current [工具]` | 当前生效版本及来源文件 |
| `runa which <命令>` | 命令实际对应的可执行文件 |
| `runa exec [--with node@20] -- <命令>` | 在指定版本环境中执行命令 |
| `runa run [--install-missing] -- <命令>` | 按项目声明的版本执行；`--install-missing` 先补装缺失版本（强制 SHA256）；docker 项目在容器中执行 |
| `runa context [--json] [--check]` | 给 Agent / AI 的只读版本上下文：项目要求、解析 / 已激活版本、冲突与建议命令；`--check` 未就绪时退出码为 1 |
| `runa project init [--mode local\|versions\|docker] [--image 镜像]` / `runa project status [--json]` | 一个项目一个环境，默认 local 隔离 |
| `runa mcp [serve\|config] --project <目录> [--read-only] [--client <客户端>]` | MCP stdio 服务，或输出 AI 客户端配置（cursor / claude-desktop / claude-code / codex / vscode / windsurf / generic） |
| `runa shell <工具> <版本> [shell]` | 打开临时子 shell 使用某版本（不写任何配置，`exit` 即恢复） |
| `runa size [工具] [--json]` / `runa clean [--json]` | 统计各版本磁盘占用 / 清理下载缓存 |
| `runa completion <bash\|zsh\|fish\|powershell\|elvish>` | 输出 shell 补全脚本 |
| `runa update [--check] [--version vX.Y.Z] [--json]` | 从 GitHub Releases 校验 SHA256 后更新 runa 命令行 |
| `runa reshim` | 重新生成 shims（如 `npm i -g` 之后） |
| `runa activate <shell>` / `runa env <shell>` | 输出钩子脚本 / 当前目录的环境变量脚本 |
| `runa detect` / `runa adopt [工具]` | 检测 / 接管系统已有运行时 |
| `runa mirror [off\|cn]` | 查看或设置下载镜像 |
| `runa proxy [off\|system\|URL]` | 查看或设置代理 |
| `runa setup [shell] [--remove]` / `runa path-init [shell]` | 写入（或移除）shell 配置文件中的激活代码块（Windows 同时写入用户 PATH） |
| `runa help [命令]` | 查看总览或某个命令的详细用法与示例 |
| `runa plugins` / `runa doctor` | 列出工具 / 诊断信息 |

## 项目环境（一个项目一个环境）

```bash
cd my-app
runa project init                 # 默认 local：版本、shims、npm/pip/go 缓存都放在 .runa/env，不影响全局
runa use node 22 && runa install  # 安装到项目自己的环境
runa run -- npm test              # 按项目声明的版本执行；缺失时可加 --install-missing
runa project init --mode versions # 只标记为项目，共享全局安装
runa project init --mode docker --image node:22-bookworm   # runa run 在容器里执行，并先校验镜像中的版本
```

`.runa/project.json` 可以提交到仓库；`.runa/.gitignore` 会自动忽略其余运行时状态。已初始化的项目拒绝被改成其它模式。

## Agent / AI 与 MCP

- `runa context --json` 输出稳定的只读 JSON（`schema_version: 1`），每个工具给出 `state`（`ready` / `missing_version` / `activation_required` / `override_conflict` / `container_unverified`）和建议命令。
- `runa mcp serve --project <目录>` 提供 MCP stdio 服务，固定绑定一个项目：`runa_context`、`runa_project_status`、`runa_list`（只读），以及 `runa_project_init`、`runa_use`、`runa_install`、`runa_run`。`--read-only` 只暴露只读工具。每次调用都在项目内以子进程运行，设置 `RUNA_PROJECT_BOUNDARY` 防止越界；通过 MCP 安装只允许 local 项目，且必须有上游 SHA256；调用有超时（1–600 秒）并支持 `notifications/cancelled`。
- `runa mcp config --project . --client cursor` 只输出配置文本（不改写客户端配置），并提示保存位置。

## 更新 runa

`runa update --check` 查询 GitHub Releases（默认 `xyunjie/runa`，可用 `RUNA_UPDATE_REPO` 指向 fork）。`runa update` 只接受比当前更新的稳定版，下载与当前平台匹配的 `runa-<tag>-<target>` 包，校验 GitHub 资产摘要（`digest`）或发布中的 `SHA256SUMS`（两者都没有则拒绝），运行 `--version` 自检后替换，失败自动回滚。桌面端以安装包发布，请从 Releases 页面下载更新。

> 发布新版本时，tag（如 `v0.2.0`）需与 `Cargo.toml` 中的版本一致，否则 `runa update` 的自检会拒绝该包。

## 桌面客户端

桌面端与 CLI 共用 `runa-core`，数据目录和设置完全一致：

- **概览**：所有运行时的当前版本、来源、已安装数量与磁盘占用，未启用自动切换时一键修复；
- **运行时详情**：已安装 / 可安装两个标签，搜索、LTS / 稳定版筛选、分页；设为全局 / 用于项目 / 卸载（带确认）；下载进度显示百分比与速度；
- **检测与接管**、**设置**（下载源、代理、Shell 激活、外观）；
- 浅色 / 深色主题跟随系统，也可手动切换。

| 浅色 | 深色 |
|---|---|
| ![](docs/screens/tool-installed-light.png) | ![](docs/screens/tool-installed-dark.png) |
| ![](docs/screens/installing-light.png) | ![](docs/screens/settings-dark.png) |

> Shell 集成和 shims 需要 `runa` 命令行程序：把它放进 PATH，或与桌面端放在同一目录（也可以用 `RUNA_CLI` 环境变量指定路径）。

## 从源码构建

命令行与核心库需要 Rust 1.88+，桌面端（Tauri）需要 Rust 1.90+（推荐通过 [rustup](https://rustup.rs) 安装 stable）。

```bash
cargo build --release -p runa-cli        # 生成 target/release/runa
cargo test -p runa-core -p runa-cli      # 单元测试
```

### 桌面端

前端是零依赖的原生 HTML / CSS / JS（`gui/ui/`），不需要 Node.js。

**Linux（Debian / Ubuntu）**

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev build-essential pkg-config libssl-dev
cargo build --release -p runa-gui        # 直接运行 target/release/runa-gui
```

Fedora：`sudo dnf install webkit2gtk4.1-devel gtk3-devel librsvg2-devel libappindicator-gtk3-devel`；Arch：`sudo pacman -S webkit2gtk-4.1 gtk3 librsvg libappindicator-gtk3`。

**macOS**：安装 Xcode Command Line Tools（`xcode-select --install`）后执行 `cargo build --release -p runa-gui`。

**Windows**：安装 Visual Studio Build Tools（勾选“使用 C++ 的桌面开发”）和 WebView2 运行时（Windows 10/11 通常已自带），然后执行 `cargo build --release -p runa-gui`。

**打包安装包**（.deb / .AppImage / .dmg / .msi）：

```bash
cargo install tauri-cli --version "^2" --locked
cd gui && cargo tauri build
```

### 界面预览与截图

`gui/preview/mock.js` 提供了模拟数据，可以在浏览器中预览界面而不启动 Tauri：

```bash
scripts/ui-screens.sh docs/screens     # 需要 Chrome / Chromium，生成浅色与深色截图
```

## 目录结构

```text
crates/runa-core   核心库：解析、安装、激活、检测、镜像代理、插件
crates/runa-cli    命令行（二进制名 runa）
gui/src-tauri      Tauri v2 桌面端（Rust 部分）
gui/ui             桌面端前端（原生 HTML/CSS/JS，无构建步骤）
gui/preview        浏览器预览用的 IPC 模拟
examples/plugins   JSON 插件示例
.github/workflows  CI 与发布流程
```

Shell 补全：

```bash
eval "$(runa completion bash)"            # zsh: runa completion zsh；fish: runa completion fish | source
runa completion powershell | Out-String | Invoke-Expression
```

数据目录（默认 `~/.runa`，可用 `RUNA_HOME` 覆盖）：

```text
.tool-versions    全局版本
settings.json     镜像 / 代理设置
installs/         各版本安装目录与安装记录（*.receipt.json）
shims/            shims
current/<工具>    指向全局版本的符号链接，方便在 IDE 中配置 JAVA_HOME 等
plugins/          JSON 插件
cache/downloads/  下载缓存
```

## 配置参考

`~/.runa/settings.json`：

```json
{
  "mirror": "cn",
  "mirror_rules": [
    { "from": "https://api.adoptium.net/", "to": "https://your-mirror.example.com/adoptium/" }
  ],
  "proxy": "socks5h://127.0.0.1:1080"
}
```

- `mirror_rules` 为自定义的 URL 前缀替换，优先于内置预设；
- 环境变量 `RUNA_MIRROR`、`RUNA_PROXY` 可临时覆盖设置；`GITHUB_TOKEN` 用于提高 GitHub API 的访问额度（JSON 插件的 `github-releases` 来源）。

## 已知限制

- Python 版本列表来自 python-build-standalone 最新一次发布，每个小版本系列只提供最新补丁版本（如 3.12.x 只有最新的那一个）。
- Java 的 `list-all` 列出的是大版本（8、11、17、21…），安装时自动选择该大版本的最新 GA 版；也可以指定完整版本号，在 Adoptium 最近 50 个发布中查找。
- 国内镜像预设不包含 Java（Adoptium API 无对应镜像），可通过 `mirror_rules` 自行配置。
- Windows 上接管已有安装需要创建符号链接的权限（开发者模式）；没有权限时会改为记录原路径，功能不受影响。
- Windows 下的 shims 是 `.cmd` 文件，适用于 cmd / PowerShell；Git Bash 请使用 `runa activate bash`。

## 许可证

MIT
