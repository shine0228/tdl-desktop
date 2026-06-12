# TDL Desktop

TDL Desktop 是一个面向 Windows 的 Telegram 下载桌面客户端。它基于 Tauri + React 构建图形界面，底层下载能力来自上游命令行项目 [tdl](https://github.com/iyear/tdl)。

本项目的目标是把常见的 `tdl` 下载流程做成更易用的桌面操作入口：管理 `tdl.exe`、登录状态、下载参数、任务历史、实时进度和消息预览，同时尽量保留 `tdl` 原有能力。

## 与上游 tdl 的关系

- 上游项目：[iyear/tdl](https://github.com/iyear/tdl)，README 中描述为 “Telegram Downloader, but more than a downloader”。
- TDL Desktop 不是上游 `tdl` 的官方 GUI，也不是对上游仓库的替代维护；它是一个独立的桌面客户端/包装器。
- 下载、登录、导出等核心 Telegram 能力仍由本机 `tdl` 执行。TDL Desktop 主要负责图形界面、参数组织、进程管理、历史记录和本地预览缓存。
- `tdl.exe` 不提交到本仓库。构建时脚本会从上游官方 GitHub Release API 下载对应 Windows 架构的 release 资产，并校验 GitHub Release asset 提供的 SHA-256 digest。
- 发布包会把下载到的 `tdl.exe` 作为 Tauri bundle 资源一起分发；用户也可以在运行时更新到用户目录中的新版 `tdl.exe`。
- `helper/tdl-helper` 会通过 Go module 依赖复用部分上游 `tdl` / `tdl/core` 包，用于登录态、对话/消息读取等辅助功能。
- 如果遇到 `tdl` 命令本身的协议、下载、账号、Telegram 限制等问题，建议同时参考上游 [tdl 文档](https://docs.iyear.me/tdl/) 和 [iyear/tdl](https://github.com/iyear/tdl) 仓库。

## 许可证

本仓库采用 **GNU Affero General Public License v3.0（AGPL-3.0）** 发布，完整文本见 [`LICENSE`](LICENSE)。

选择 AGPL-3.0 的原因：

- 上游 [iyear/tdl](https://github.com/iyear/tdl) 的许可证为 AGPL-3.0。
- 本项目构建/发布时会分发上游 `tdl.exe`。
- 本项目的 `helper/tdl-helper` 直接依赖上游 `github.com/iyear/tdl` 和 `github.com/iyear/tdl/core` Go 模块。

如果你二次分发 TDL Desktop 或其修改版，请注意：

1. 保留本仓库和上游 `tdl` 的许可证声明与版权/归属信息。
2. 随二进制发布提供对应源码，或提供清晰、可访问的源码获取方式。
3. 发布包含 `tdl.exe` 的安装包时，建议在 release 说明中记录所包含的 `tdl` 版本、上游 release 链接和对应源码链接。
4. 不要把本项目描述为上游 `tdl` 官方项目，除非取得上游维护者授权。

以上仅是项目合规说明，不构成法律意见。

## 功能

- 内置 Windows 版 `tdl.exe` 到 Tauri bundle。
- 启动时优先使用内置 `tdl.exe`，用户主动更新后使用用户目录里的新版。
- Rust 后端统一管理下载进程，Windows 下隐藏子进程窗口。
- 支持链接下载、Telegram Desktop 导出的 JSON、原始 `tdl` 参数。
- 支持按对话浏览最近消息，识别图片、视频、音频和文件消息，并按需缓存图片/视频预览。
- 普通 `t.me/<频道>/<消息ID>` 链接粘贴后可自动读取消息文字预览。
- 支持 `--group`、扩展名过滤、续传、重启、跳过同名同大小、模板、takeout 等常用参数。
- 支持 Telegram Desktop 登录态复用和 QR 登录流程。
- 下载历史保存到 `~/.tdl-desktop/history.json`，失败信息会落盘。
- 设置页可查看 `tdl` 路径/版本、配置命名空间、存储参数、日志目录和诊断日志包。

## 快速开始

1. 下载或构建 TDL Desktop。
2. 打开应用，确认顶部显示 `tdl` 可用。
3. 在右侧 Telegram 登录区域点击“检查”。
4. 如果未登录，使用“连接 Desktop”复用 Telegram Desktop 登录态，或使用 QR 登录。
5. 选择“链接下载”“JSON 导入”或“对话浏览”。
6. 填写链接/JSON 文件/对话信息，选择本地下载目录。
7. 按需调整并发、线程、过滤、命名模板等参数。
8. 点击开始下载，在右侧查看实时进度和最新 `tdl` 输出。

更完整的操作说明见 [`docs/tdl-desktop-usage.md`](docs/tdl-desktop-usage.md)。

## 项目结构

```text
.
├── src/                    # React 前端
├── src-tauri/              # Tauri/Rust 后端和打包配置
│   ├── src/                # Tauri 命令、进程管理、tdl 集成
│   └── resources/          # 构建时放入 tdl.exe / tdl-helper.exe，不提交二进制
├── helper/tdl-helper/      # Go 辅助程序，复用 tdl 登录态和消息读取能力
├── scripts/                # 构建、下载 tdl、收集 release 的辅助脚本
├── docs/                   # 用户操作文档
├── public/                 # 前端静态资源
├── build.bat               # 发布构建入口
└── run.bat                 # 开发运行入口
```

## 环境要求

- Windows 10/11。
- Node.js 22+。
- npm 11+。
- Rust/Cargo 1.77.2+。
- Visual Studio Build Tools 2022 C++ 工具链。
- Windows WebView2 Runtime。
- 构建 `tdl-helper` 时需要 Go 工具链。

如果 Rust/Cargo 没有加入 `PATH`，可以在运行脚本前设置 `RUST_ROOT` 环境变量指向 Rust 工具链根目录；脚本会优先使用 `%RUST_ROOT%\cargo\bin`。

## 开发

```bat
npm ci
run.bat
```

也可以直接执行：

```bat
npm run tauri:dev
```

`npm run tauri:dev` 会先执行 `npm run fetch:tdl`，确保 `src-tauri\resources\tdl.exe` 存在。

## 构建

```bat
build.bat
```

或直接执行：

```bat
npm ci
npm run tauri:build
```

构建完成后，发版产物会自动收集到：

```text
release\
```

Tauri 原始构建产物仍位于 `src-tauri\target\release\`，但发版时以根目录 `release\` 为准。

## 常用脚本

```bat
npm run fetch:tdl       # 下载上游 release 中的 Windows tdl.exe
npm run build:helper    # 构建 Go 辅助程序到 src-tauri\resources\tdl-helper.exe
npm run build           # TypeScript 检查并构建前端
npm run lint            # 前端 lint
npm run lint:rust       # Rust clippy
npm run test:rust       # Rust 测试
npm run tauri:build     # 下载 tdl、构建 Tauri、收集 release
```

## tdl 二进制来源

`tdl.exe` 不提交到 Git。构建前会自动执行：

```bat
npm run fetch:tdl
```

该脚本默认请求上游官方 release API：

```text
https://api.github.com/repos/iyear/tdl/releases/latest
```

然后按当前 Node.js 架构选择 release 资产：

- x64：`tdl_Windows_64bit.zip`
- ia32：`tdl_Windows_32bit.zip`
- arm64：`tdl_Windows_arm64.zip`
- arm：`tdl_Windows_armv7.zip`

下载完成后脚本会校验 release asset 的 SHA-256 digest，解压并写入：

```text
src-tauri\resources\tdl.exe
```

随后由 Tauri 打进桌面端 bundle。

### 使用自定义 tdl 镜像

如果 GitHub 访问不稳定，或需要临时使用自建 release 镜像，可以设置 `TDL_MIRROR`。该环境变量优先级最高，不需要修改仓库代码：

```bat
set TDL_MIRROR=https://api.github.com/repos/myorg/tdl-mirror/releases/latest
npm run fetch:tdl
```

建议仅在本机或 CI 环境变量中配置镜像，不要把个人 fork 或临时镜像地址硬编码到仓库。

## tdl 登录

首次下载私有内容或浏览对话前，需要让 `tdl` 登录 Telegram 账号。开发环境可执行：

```bat
src-tauri\resources\tdl.exe login
```

发布版可以通过应用内的 Telegram 登录区域完成登录，也可以使用内置 `tdl.exe` 或用户更新后的 `~/.tdl-desktop/bin/tdl.exe` 完成登录。

常见登录方式：

- **连接 Desktop**：复用本机 Telegram Desktop 登录态，适合已经安装并登录 Telegram Desktop 的用户。
- **QR 登录**：没有 Telegram Desktop 登录态，或 Desktop 连接失败时使用。
- **命名空间**：设置页可配置 `tdl` namespace，用于隔离不同登录态和数据。

## 本地数据与隐私

TDL Desktop 默认只在本机保存运行所需数据：

- 下载历史：`~/.tdl-desktop/history.json`
- 日志：通常位于 `~/.tdl-desktop/logs`
- 预览缓存：`~/.tdl-desktop/previews`
- 用户更新后的 `tdl.exe`：`~/.tdl-desktop/bin/tdl.exe`

诊断日志包需要用户手动生成和提交，应用不会自动上传日志或下载内容。

## 常见问题

### 顶部显示 tdl 不可用怎么办？

先点击刷新。如果仍不可用，请确认发布包完整、`resources` 目录没有被删除、杀毒软件没有隔离 `tdl.exe`。开发环境可重新执行：

```bat
npm run fetch:tdl
```

### 下载私有频道或受限内容失败怎么办？

确认当前 `tdl` 账号已经登录，并且账号本身有权限访问对应频道、群组或消息。受限内容能否下载取决于 Telegram 权限、账号状态和上游 `tdl` 支持情况。

### 预览失败会影响下载吗？

通常不会。预览需要登录态和网络可用，大文件或受限内容可能预览失败，但仍可继续创建下载任务。最终下载结果以 `tdl` 执行输出为准。

### 可以直接使用原始 tdl 参数吗？

可以。原始参数模式会把参数直接传给 `tdl`，适合熟悉命令行的高级用户。普通下载建议优先使用链接下载、JSON 导入或对话浏览。

### 本项目是否修改了上游 tdl？

本仓库不提交上游 `tdl.exe`，构建时从官方 release 下载；默认也不修改上游 `tdl` 源码。桌面端通过进程调用和辅助程序复用 `tdl` 能力。如果未来引入对上游源码的补丁，应在 README/release 中明确记录补丁和对应源码。

## 贡献

欢迎提交 issue 或 pull request。建议在提交前运行：

```bat
npm run lint
npm run lint:rust
npm run test:rust
```

涉及上游 `tdl` 行为的问题，请尽量附上 `tdl` 版本、TDL Desktop 版本、复现步骤和相关日志。确认是上游 CLI 行为时，也可以到 [iyear/tdl](https://github.com/iyear/tdl) 反馈。
