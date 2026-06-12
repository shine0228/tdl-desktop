# Project Instructions

本仓库的主客户端是 Tauri + React。

## 工作原则

- 默认只修改当前任务需要的最小范围。
- 先读相关代码，再改实现。
- 不删除构建产物或大文件，除非用户明确要求。
- Windows 子进程调用必须通过 Rust 后端统一处理，不能在前端直接执行命令。
- `tdl.exe` 不提交到 Git；构建前通过 `npm run fetch:tdl` 下载到 `src-tauri/resources/tdl.exe`，再由 Tauri 打进 bundle。

## tdl 下载源

构建时（`scripts/fetch-tdl.mjs`）和运行时自动更新（`src-tauri/src/tdl.rs`）默认使用官方 release API：

1. `https://api.github.com/repos/iyear/tdl/releases/latest`

如需临时使用自建镜像，优先通过 `TDL_MIRROR` 环境变量配置，不要把个人 fork 地址硬编码到仓库。

### TDL_MIRROR 环境变量

设置 `TDL_MIRROR` 环境变量可临时前置一个自定义镜像源（优先级最高），无需修改代码：

```bat
set TDL_MIRROR=https://api.github.com/repos/myorg/tdl-mirror/releases/latest
npm run fetch:tdl
```

## 常用命令

```bat
npm ci
npm run build
npm run tauri:build
```

开发运行：

```bat
run.bat
```

发布构建：

```bat
build.bat
```

## 目录约定

- `src/`: React 前端
- `src-tauri/`: Tauri/Rust 后端和打包配置
- `scripts/`: 构建辅助脚本
- `release/`: 本地发版产物目录，不提交到 Git
