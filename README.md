# TDL Desktop

TDL Desktop 是基于 Tauri + React 的 Telegram 下载客户端，底层调用 [tdl](https://github.com/iyear/tdl)。

## 功能

- 内置 Windows 版 `tdl.exe` 到 Tauri bundle
- 启动时优先使用内置 `tdl.exe`，用户主动更新后使用用户目录里的新版
- Rust 后端统一管理下载进程，Windows 下隐藏子进程窗口
- 支持链接下载、Telegram Desktop 导出的 JSON、原始 tdl 参数
- 支持按对话浏览最近消息，识别图片、视频、音频和文件消息，并按需缓存图片/视频预览
- 普通 `t.me/<频道>/<消息ID>` 链接粘贴后可自动读取消息文字预览
- 支持 `--group`、扩展名过滤、续传、重启、跳过同名同大小、模板、takeout 等常用参数
- 下载历史保存到 `~/.tdl-desktop/history.json`，失败信息会落盘

## 项目结构

```text
.
├── src/                 # React 前端
├── src-tauri/           # Tauri/Rust 后端和打包配置
├── scripts/             # 构建辅助脚本
├── public/              # 前端静态资源
├── release/             # 本地发版产物，不提交 Git
├── build.bat            # 发布构建入口
└── run.bat              # 开发运行入口
```

## 环境要求

- Node.js 22+
- npm 11+
- Rust/Cargo 1.77.2+
- Visual Studio Build Tools 2022 C++ 工具链
- Windows WebView2 Runtime

如果 Rust/Cargo 没有加入 `PATH`，可以在运行脚本前设置 `RUST_ROOT` 环境变量指向 Rust 工具链根目录；脚本会优先使用 `%RUST_ROOT%\cargo\bin`。

## 开发

```bat
npm install
run.bat
```

也可以直接执行：

```bat
npm run tauri:dev
```

## 构建

```bat
build.bat
```

或直接执行：

```bat
npm install
npm run tauri:build
```

构建完成后，发版产物会自动收集到：

```text
release\
```

Tauri 原始构建产物仍位于 `src-tauri\target\release\`，但发版时以根目录 `release\` 为准。

## tdl 二进制

`tdl.exe` 不提交到 Git。构建前会自动执行：

```bat
npm run fetch:tdl
```

该脚本会下载 Windows 版 `tdl.exe` 到：

```text
src-tauri\resources\tdl.exe
```

随后由 Tauri 打进桌面端 bundle。

## tdl 登录

首次下载前仍需要登录 Telegram 账号。开发环境可执行：

```bat
src-tauri\resources\tdl.exe login
```

发布版可通过内置 `tdl.exe` 或用户更新后的 `~/.tdl-desktop/bin/tdl.exe` 完成登录。
