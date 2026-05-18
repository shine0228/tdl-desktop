import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AlertCircle,
  Bot,
  CheckCircle2,
  ChevronDown,
  Download,
  FileJson,
  FolderOpen,
  Link as LinkIcon,
  ListChecks,
  LogIn,
  LoaderCircle,
  MessageSquareText,
  Play,
  QrCode,
  RefreshCcw,
  RotateCw,
  Settings2,
  ShieldCheck,
  Square,
  Terminal,
  Trash2,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  AppConfig,
  AppState,
  ChatDownloadRequest,
  ChatInfo,
  DownloadEvent,
  DownloadFileProgress,
  DownloadRecord,
  DownloadRequest,
  DownloadStarted,
  DownloadStatus,
  MessageInfo,
  LoginEvent,
  LoginMethod,
  LoginStarted,
  LoginStatus,
  LinkPreview,
  SourceMode,
  TdlInfo,
} from "./types";

const DEFAULT_CONFIG: AppConfig = {
  lastDirectory: "",
  limit: 4,
  threads: 4,
  pool: 8,
  tdlOverridePath: null,
};

const statusLabel: Record<DownloadStatus, string> = {
  downloading: "下载中",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

const modeLabel: Record<SourceMode, string> = {
  links: "链接",
  json: "JSON",
  raw: "原始参数",
  chat: "对话",
};

type PreviewState =
  | { status: "idle" }
  | { status: "loading"; link: string }
  | { status: "ready"; link: string; preview: LinkPreview }
  | { status: "error"; link: string; error: string };

function inTauri() {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

function splitLines(value: string) {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function formatDate(value?: string | null) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function formatFileSize(value?: number | null) {
  if (!value || value <= 0) return "未知大小";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 10 || unit === 0 ? Math.round(size) : size.toFixed(1)} ${units[unit]}`;
}

function tdlSourceLabel(info: TdlInfo | null) {
  if (!info || !info.available) return "tdl 不可用";
  const source =
    info.source === "bundled"
      ? "内置"
      : info.source === "updated"
        ? "用户更新"
        : "PATH";
  return `${source} ${info.version ?? ""}`.trim();
}

function buildLocalPreview(request: DownloadRequest) {
  if (request.mode === "chat") {
    return "tdl chat export ... && tdl download -f <selected-messages.json>";
  }

  if (request.mode === "raw") {
    return `tdl ${request.rawArgs.trim()}`;
  }

  const args = [
    "tdl",
    "download",
    "-d",
    quoteArg(request.directory || "<下载目录>"),
    "-l",
    String(request.limit),
    "-t",
    String(request.threads),
    "--pool",
    String(request.pool),
  ];

  if (request.mode === "links") {
    request.links.forEach((link) => args.push("-u", quoteArg(link)));
  } else {
    request.files.forEach((file) => args.push("-f", quoteArg(file)));
  }

  if (request.group) args.push("--group");
  if (request.include.trim()) args.push("-i", quoteArg(request.include.trim()));
  if (request.exclude.trim()) args.push("-e", quoteArg(request.exclude.trim()));
  if (request.template.trim()) args.push("--template", quoteArg(request.template.trim()));
  if (request.skipSame) args.push("--skip-same");
  if (request.continueLast) args.push("--continue");
  if (request.restart) args.push("--restart");
  if (request.desc) args.push("--desc");
  if (request.takeout) args.push("--takeout");
  if (request.rewriteExt) args.push("--rewrite-ext");

  return args.join(" ");
}

function quoteArg(value: string) {
  if (!value || /\s/.test(value)) {
    return `"${value.replaceAll('"', '\\"')}"`;
  }
  return value;
}

function firstPreviewCandidate(value: string) {
  return splitLines(value).find((line) =>
    /^(https?:\/\/)?(www\.)?(t\.me|telegram\.me)\//i.test(line),
  );
}

function App() {
  const [config, setConfig] = useState<AppConfig>(DEFAULT_CONFIG);
  const [tdl, setTdl] = useState<TdlInfo | null>(null);
  const [history, setHistory] = useState<DownloadRecord[]>([]);
  const [mode, setMode] = useState<SourceMode>("links");
  const [linksText, setLinksText] = useState("");
  const [filesText, setFilesText] = useState("");
  const [rawArgs, setRawArgs] = useState("download ");
  const [directory, setDirectory] = useState("");
  const [group, setGroup] = useState(true);
  const [include, setInclude] = useState("");
  const [exclude, setExclude] = useState("");
  const [template, setTemplate] = useState("");
  const [skipSame, setSkipSame] = useState(true);
  const [continueLast, setContinueLast] = useState(false);
  const [restart, setRestart] = useState(false);
  const [desc, setDesc] = useState(false);
  const [takeout, setTakeout] = useState(false);
  const [rewriteExt, setRewriteExt] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(true);
  const [runningTaskId, setRunningTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [logsOpen, setLogsOpen] = useState(false);
  const [linkPreview, setLinkPreview] = useState<PreviewState>({ status: "idle" });
  const [progress, setProgress] = useState<number | null>(null);
  const [fileProgresses, setFileProgresses] = useState<DownloadFileProgress[]>([]);
  const [loginStatus, setLoginStatus] = useState<LoginStatus>({
    loggedIn: false,
    message: "尚未检查登录状态",
    detail: null,
  });
  const [loginChecking, setLoginChecking] = useState(false);
  const [loginRunning, setLoginRunning] = useState(false);
  const [loginQr, setLoginQr] = useState("");
  const [loginLogs, setLoginLogs] = useState<string[]>([]);
  const [desktopPath, setDesktopPath] = useState("");
  const [desktopPasscode, setDesktopPasscode] = useState("");
  const [chats, setChats] = useState<ChatInfo[]>([]);
  const [chatSearch, setChatSearch] = useState("");
  const [chatLoading, setChatLoading] = useState(false);
  const [selectedChat, setSelectedChat] = useState<ChatInfo | null>(null);
  const [messages, setMessages] = useState<MessageInfo[]>([]);
  const [messagesLoading, setMessagesLoading] = useState(false);
  const [selectedMessageIds, setSelectedMessageIds] = useState<Set<number>>(new Set());
  const [messageCount, setMessageCount] = useState(50);
  const [message, setMessage] = useState("正在加载");
  const [busy, setBusy] = useState(false);

  const configSaveTimer = useRef<number | null>(null);
  const pendingConfigRef = useRef<AppConfig | null>(null);
  const previewRequestSeq = useRef(0);

  const flushConfig = useCallback(async () => {
    if (configSaveTimer.current !== null) {
      window.clearTimeout(configSaveTimer.current);
      configSaveTimer.current = null;
    }
    const pending = pendingConfigRef.current;
    pendingConfigRef.current = null;
    if (!pending || !inTauri()) return;
    try {
      await invoke<AppConfig>("save_config", { config: pending });
    } catch (error) {
      setMessage(String(error));
    }
  }, []);

  const persistConfig = useCallback(
    (next: AppConfig, options?: { immediate?: boolean }) => {
      setConfig(next);
      pendingConfigRef.current = next;
      if (!inTauri()) return;
      if (options?.immediate) {
        void flushConfig();
        return;
      }
      if (configSaveTimer.current !== null) {
        window.clearTimeout(configSaveTimer.current);
      }
      configSaveTimer.current = window.setTimeout(() => {
        configSaveTimer.current = null;
        void flushConfig();
      }, 400);
    },
    [flushConfig],
  );

  useEffect(
    () => () => {
      if (configSaveTimer.current !== null) {
        window.clearTimeout(configSaveTimer.current);
        // 卸载前同步落盘最近一次配置变更
        void flushConfig();
      }
    },
    [flushConfig],
  );

  useEffect(() => {
    void loadState();

    if (!inTauri()) return;

    let cancelled = false;
    let unlistenDownload: (() => void) | undefined;
    let unlistenLogin: (() => void) | undefined;

    listen<DownloadEvent>("download-event", (event) => {
      handleDownloadEvent(event.payload);
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlistenDownload = fn;
        }
      })
      .catch((error) => {
        if (!cancelled) {
          console.error("订阅 download-event 失败", error);
        }
      });

    listen<LoginEvent>("login-event", (event) => {
      handleLoginEvent(event.payload);
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlistenLogin = fn;
        }
      })
      .catch((error) => {
        if (!cancelled) {
          console.error("订阅 login-event 失败", error);
        }
      });

    return () => {
      cancelled = true;
      unlistenDownload?.();
      unlistenLogin?.();
    };
  }, []);

  useEffect(() => {
    if (!config.lastDirectory) return;
    setDirectory((current) => current || config.lastDirectory);
  }, [config.lastDirectory]);

  useEffect(() => {
    const candidate = mode === "links" ? firstPreviewCandidate(linksText) : undefined;
    previewRequestSeq.current += 1;
    const seq = previewRequestSeq.current;

    if (!candidate) {
      setLinkPreview({ status: "idle" });
      return;
    }

    if (!inTauri()) {
      setLinkPreview({ status: "idle" });
      return;
    }

    if (tdl && !tdl.available) {
      setLinkPreview({ status: "error", link: candidate, error: "tdl 不可用，无法读取消息预览。" });
      return;
    }

    setLinkPreview({ status: "loading", link: candidate });
    const timer = window.setTimeout(() => {
      invoke<LinkPreview>("preview_link", { link: candidate })
        .then((preview) => {
          if (previewRequestSeq.current === seq) {
            setLinkPreview({ status: "ready", link: candidate, preview });
          }
        })
        .catch((error) => {
          if (previewRequestSeq.current === seq) {
            setLinkPreview({ status: "error", link: candidate, error: String(error) });
          }
        });
    }, 700);

    return () => {
      window.clearTimeout(timer);
    };
  }, [linksText, mode, tdl]);

  const request = useMemo<DownloadRequest>(
    () => ({
      mode,
      directory,
      links: splitLines(linksText),
      files: splitLines(filesText),
      rawArgs,
      limit: config.limit,
      threads: config.threads,
      pool: config.pool,
      group,
      include,
      exclude,
      template,
      skipSame,
      continueLast,
      restart,
      desc,
      takeout,
      rewriteExt,
    }),
    [
      config.limit,
      config.pool,
      config.threads,
      continueLast,
      desc,
      directory,
      exclude,
      filesText,
      group,
      include,
      linksText,
      mode,
      rawArgs,
      restart,
      rewriteExt,
      skipSame,
      takeout,
      template,
    ],
  );

  const commandPreview = useMemo(() => buildLocalPreview(request), [request]);

  const filteredChats = useMemo(() => {
    const keyword = chatSearch.trim().toLowerCase();
    if (!keyword) return chats;
    return chats.filter((chat) =>
      [chat.name, chat.username ?? "", chat.chatType, String(chat.id)]
        .join(" ")
        .toLowerCase()
        .includes(keyword),
    );
  }, [chatSearch, chats]);

  async function loadState() {
    if (!inTauri()) {
      setMessage("前端预览模式");
      setTdl({ available: false, source: "missing", path: null, version: null });
      setConfig({ ...DEFAULT_CONFIG, lastDirectory: "D:\\Downloads" });
      return;
    }

    try {
      const state = await invoke<AppState>("get_app_state");
      setConfig(state.config);
      setDirectory(state.config.lastDirectory);
      setHistory(state.history);
      setTdl(state.tdl);
      setMessage(state.tdl.available ? "就绪" : "tdl 不可用");
      if (state.tdl.available) {
        void refreshLoginStatus();
      } else {
        setLoginStatus({
          loggedIn: false,
          message: "tdl 不可用，无法检查 Telegram 登录状态。",
          detail: null,
        });
      }
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function refreshLoginStatus() {
    if (!inTauri()) return;
    setLoginChecking(true);
    try {
      const status = await invoke<LoginStatus>("check_login_status");
      setLoginStatus(status);
    } catch (error) {
      setLoginStatus({
        loggedIn: false,
        message: "无法检查 Telegram 登录状态。",
        detail: String(error),
      });
    } finally {
      setLoginChecking(false);
    }
  }

  function handleDownloadEvent(event: DownloadEvent) {
    if (event.kind === "output" && event.line) {
      if (event.fileProgress) {
        mergeFileProgress(event.fileProgress);
      } else {
        setLogs((current) => [...current.slice(-120), event.line as string]);
      }
      if (typeof event.progress === "number") {
        setProgress(Math.max(0, Math.min(100, event.progress)));
      }
      return;
    }

    if (event.kind === "complete") {
      setRunningTaskId((current) => (current === event.taskId ? null : current));
      setProgress(event.status === "completed" ? 100 : null);
      if (event.status === "completed") {
        setFileProgresses((current) =>
          current.map((item) => ({ ...item, progress: 100, done: true })),
        );
      }
      setMessage(event.message ?? "");
      setHistory((current) =>
        current.map((record) =>
          event.recordIds.includes(record.id)
            ? {
                ...record,
                status: event.status ?? record.status,
                completedAt: event.completedAt,
                error: event.error,
              }
            : record,
        ),
      );
    }
  }

  function mergeFileProgress(next: DownloadFileProgress) {
    const progress = Math.max(0, Math.min(100, next.progress));
    setFileProgresses((current) => {
      const index = current.findIndex((item) => item.key === next.key);
      const normalized = { ...next, progress, done: next.done || progress >= 99.9 };
      if (index === -1) {
        return [...current, normalized];
      }
      return current.map((item, itemIndex) =>
        itemIndex === index
          ? {
              ...item,
              ...normalized,
              progress: Math.max(item.progress, normalized.progress),
              done: item.done || normalized.done,
            }
          : item,
      );
    });
  }

  function handleLoginEvent(event: LoginEvent) {
    if (event.kind === "output" && event.line) {
      setLoginLogs((current) => [...current.slice(-80), event.line as string]);
      return;
    }

    if (event.kind === "qr" && event.qr) {
      setLoginQr(event.qr);
      return;
    }

    if (event.kind === "complete") {
      setLoginRunning(false);
      const completed = event.status === "completed";
      setLoginStatus({
        loggedIn: completed,
        message: event.message ?? (completed ? "登录完成" : "登录失败"),
        detail: event.error,
      });
      setMessage(event.message ?? "");
      if (completed) {
        void refreshLoginStatus();
      }
    }
  }

  async function saveConfig(next: AppConfig) {
    persistConfig(next, { immediate: true });
  }

  async function pickDirectory() {
    if (!inTauri()) return;
    const picked = await invoke<string | null>("pick_directory");
    if (picked) {
      setDirectory(picked);
      await saveConfig({ ...config, lastDirectory: picked });
    }
  }

  async function loadChats() {
    if (!inTauri()) return;
    setChatLoading(true);
    setMessage("正在读取对话列表");
    try {
      const items = await invoke<ChatInfo[]>("list_chats");
      setChats(items);
      setMessage(`已加载 ${items.length} 个对话`);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setChatLoading(false);
    }
  }

  async function loadMessages(chat: ChatInfo) {
    if (!inTauri()) return;
    setSelectedChat(chat);
    setMessages([]);
    setSelectedMessageIds(new Set());
    setMessagesLoading(true);
    setMessage(`正在读取 ${chat.name} 的最近 ${messageCount} 条消息`);
    try {
      const items = await invoke<MessageInfo[]>("export_chat_messages", {
        chatId: String(chat.id),
        count: messageCount,
      });
      setMessages(items);
      setMessage(`已读取 ${items.length} 条消息`);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setMessagesLoading(false);
    }
  }

  function toggleMessage(id: number) {
    setSelectedMessageIds((current) => {
      const next = new Set(current);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  function toggleAllMessages() {
    setSelectedMessageIds((current) => {
      if (current.size === messages.length) return new Set();
      return new Set(messages.map((item) => item.id));
    });
  }

  async function startDownload() {
    if (runningTaskId) return;
    if (!directory.trim() && mode !== "raw") {
      setMessage("请选择下载目录");
      return;
    }
    if (mode === "chat" && (!selectedChat || selectedMessageIds.size === 0)) {
      setMessage("请选择对话和至少一条消息");
      return;
    }

    setBusy(true);
    setLogs([]);
    setProgress(0);
    setFileProgresses([]);
    setMessage("启动下载");

    try {
      const started = mode === "chat"
        ? await invoke<DownloadStarted>("download_from_chat", {
            request: {
              chatId: String(selectedChat!.id),
              chatName: selectedChat!.name,
              messageIds: Array.from(selectedMessageIds).sort((a, b) => a - b),
              directory,
              limit: config.limit,
              threads: config.threads,
              pool: config.pool,
              group,
              include,
              exclude,
              template,
              skipSame,
              continueLast,
              restart,
              desc,
              takeout,
              rewriteExt,
            } satisfies ChatDownloadRequest,
          })
        : await invoke<DownloadStarted>("start_download", { request });
      setRunningTaskId(started.taskId);
      setHistory((current) => [...started.records, ...current]);
      setLogs([started.commandPreview]);
      setMessage("下载中");
      await saveConfig({ ...config, lastDirectory: directory });
    } catch (error) {
      setProgress(null);
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function cancelDownload() {
    if (!runningTaskId || !inTauri()) return;
    await invoke("cancel_download", { taskId: runningTaskId });
    setMessage("正在取消");
  }

  async function updateTdl() {
    if (!inTauri()) return;
    setBusy(true);
    setMessage("正在更新 tdl");
    try {
      const info = await invoke<TdlInfo>("update_tdl");
      setTdl(info);
      setMessage(info.available ? "tdl 已更新" : "tdl 不可用");
      await loadState();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function startLogin(method: LoginMethod) {
    if (!inTauri() || loginRunning) return;
    setLoginRunning(true);
    setLoginQr("");
    setLoginLogs([]);
    setLoginStatus({
      loggedIn: false,
      message: method === "desktop" ? "正在连接 Telegram Desktop" : "正在生成 QR 登录码",
      detail: null,
    });

    try {
      const request = {
        method,
        desktopPath: desktopPath.trim() || null,
        passcode: desktopPasscode || null,
      };
      const started = await invoke<LoginStarted>("start_login", { request });
      setLoginLogs([
        started.method === "desktop"
          ? "正在通过 Telegram Desktop 接入 tdl..."
          : "请使用 Telegram 手机端扫描下方二维码...",
      ]);
    } catch (error) {
      setLoginRunning(false);
      setLoginStatus({
        loggedIn: false,
        message: "启动登录失败",
        detail: String(error),
      });
      setMessage(String(error));
    }
  }

  async function cancelLogin() {
    if (!inTauri() || !loginRunning) return;
    await invoke("cancel_login");
    setLoginStatus({
      loggedIn: false,
      message: "正在取消登录",
      detail: null,
    });
  }

  async function clearHistory() {
    if (!inTauri()) {
      setHistory([]);
      return;
    }
    await invoke("clear_history");
    setHistory([]);
  }

  const running = Boolean(runningTaskId);
  const completed = history.filter((item) => item.status === "completed").length;
  const failed = history.filter((item) => item.status === "failed").length;
  const latestLog = logs.at(-1) ?? (running ? "等待 tdl 输出" : "等待任务输出");
  const progressValue = progress ?? 0;
  const completedFiles = fileProgresses.filter((item) => item.done || item.progress >= 99.9).length;
  const progressSummary = fileProgresses.length
    ? `${completedFiles}/${fileProgresses.length} 个文件`
    : progress !== null
      ? `${Math.round(progress)}%`
      : running
        ? "运行中"
        : "-";

  return (
    <main className="app-shell">
      <section className="workbench">
        <header className="topbar">
          <div>
            <h1>TDL Desktop</h1>
            <div className="status-line">
              <span className={`status-dot ${tdl?.available ? "ready" : "error"}`} />
              <span>{tdlSourceLabel(tdl)}</span>
              {tdl?.path ? <code>{tdl.path}</code> : null}
            </div>
          </div>
          <div className="topbar-actions">
            <button className="ghost-button" onClick={loadState} disabled={busy}>
              <RefreshCcw size={16} />
              刷新
            </button>
            <button className="ghost-button" onClick={updateTdl} disabled={busy}>
              <RotateCw size={16} />
              更新 tdl
            </button>
          </div>
        </header>

        <div className="content-grid">
          <section className="task-panel">
            <div className="section-header">
              <h2>下载任务</h2>
              <span>{message}</span>
            </div>

            <div className="segmented">
              <button className={mode === "links" ? "active" : ""} onClick={() => setMode("links")}>
                <LinkIcon size={16} />
                链接
              </button>
              <button className={mode === "json" ? "active" : ""} onClick={() => setMode("json")}>
                <FileJson size={16} />
                JSON
              </button>
              <button className={mode === "raw" ? "active" : ""} onClick={() => setMode("raw")}>
                <Terminal size={16} />
                原始
              </button>
              <button className={mode === "chat" ? "active" : ""} onClick={() => setMode("chat")}>
                <Bot size={16} />
                对话
              </button>
            </div>

            {mode === "links" ? (
              <>
                <label className="field">
                  <span>消息链接</span>
                  <textarea
                    value={linksText}
                    onChange={(event) => setLinksText(event.target.value)}
                    spellCheck={false}
                    placeholder="https://t.me/channel/123"
                  />
                </label>
                <LinkPreviewPanel state={linkPreview} />
              </>
            ) : null}

            {mode === "json" ? (
              <label className="field">
                <span>导出文件路径</span>
                <textarea
                  value={filesText}
                  onChange={(event) => setFilesText(event.target.value)}
                  spellCheck={false}
                  placeholder="D:\\Downloads\\result.json"
                />
              </label>
            ) : null}

            {mode === "raw" ? (
              <label className="field">
                <span>tdl 参数</span>
                <textarea
                  value={rawArgs}
                  onChange={(event) => setRawArgs(event.target.value)}
                  spellCheck={false}
                  placeholder="download -u https://t.me/tdl/1 --group"
                />
              </label>
            ) : null}

            {mode === "chat" ? (
              <ChatBrowser
                chats={filteredChats}
                selectedChat={selectedChat}
                messages={messages}
                selectedMessageIds={selectedMessageIds}
                chatSearch={chatSearch}
                messageCount={messageCount}
                loadingChats={chatLoading}
                loadingMessages={messagesLoading}
                onSearchChange={setChatSearch}
                onMessageCountChange={setMessageCount}
                onLoadChats={() => void loadChats()}
                onSelectChat={(chat) => void loadMessages(chat)}
                onToggleMessage={toggleMessage}
                onToggleAll={toggleAllMessages}
              />
            ) : null}

            {mode !== "raw" ? (
              <div className="directory-row">
                <label className="field compact">
                  <span>下载目录</span>
                  <input value={directory} onChange={(event) => setDirectory(event.target.value)} />
                </label>
                <button className="icon-button" onClick={pickDirectory} title="选择目录">
                  <FolderOpen size={18} />
                </button>
              </div>
            ) : null}

            <div className="number-grid">
              <label className="field compact">
                <span>任务并发</span>
                <input
                  type="number"
                  min={1}
                  max={32}
                  value={config.limit}
                  onChange={(event) =>
                    persistConfig({ ...config, limit: Number(event.target.value) })
                  }
                  onBlur={() => void flushConfig()}
                />
              </label>
              <label className="field compact">
                <span>单文件线程</span>
                <input
                  type="number"
                  min={1}
                  max={32}
                  value={config.threads}
                  onChange={(event) =>
                    persistConfig({ ...config, threads: Number(event.target.value) })
                  }
                  onBlur={() => void flushConfig()}
                />
              </label>
              <label className="field compact">
                <span>DC 池</span>
                <input
                  type="number"
                  min={0}
                  max={64}
                  value={config.pool}
                  onChange={(event) =>
                    persistConfig({ ...config, pool: Number(event.target.value) })
                  }
                  onBlur={() => void flushConfig()}
                />
              </label>
            </div>

            <button className="advanced-toggle" onClick={() => setAdvancedOpen((value) => !value)}>
              <Settings2 size={16} />
              高级参数
              <ChevronDown size={16} className={advancedOpen ? "open" : ""} />
            </button>

            {advancedOpen ? (
              <div className="advanced-grid">
                <Toggle label="群组模式" checked={group} onChange={setGroup} />
                <Toggle label="跳过同名同大小" checked={skipSame} onChange={setSkipSame} />
                <Toggle label="续传" checked={continueLast} onChange={setContinueLast} />
                <Toggle label="重启任务" checked={restart} onChange={setRestart} />
                <Toggle label="倒序" checked={desc} onChange={setDesc} />
                <Toggle label="Takeout" checked={takeout} onChange={setTakeout} />
                <Toggle label="重写扩展名" checked={rewriteExt} onChange={setRewriteExt} />
                <label className="field compact">
                  <span>包含扩展名</span>
                  <input value={include} onChange={(event) => setInclude(event.target.value)} placeholder="mp4,jpg" />
                </label>
                <label className="field compact">
                  <span>排除扩展名</span>
                  <input value={exclude} onChange={(event) => setExclude(event.target.value)} placeholder="tmp,part" />
                </label>
                <label className="field compact wide">
                  <span>文件名模板</span>
                  <input value={template} onChange={(event) => setTemplate(event.target.value)} />
                </label>
              </div>
            ) : null}

            <div className="command-preview">
              <Terminal size={15} />
              <code>{commandPreview}</code>
            </div>

            <div className="action-row">
              <button className="primary-button" onClick={startDownload} disabled={busy || running}>
                <Play size={17} />
                开始下载
              </button>
              <button className="danger-button" onClick={cancelDownload} disabled={!running}>
                <Square size={16} />
                取消
              </button>
            </div>
          </section>

          <section className="side-panel">
            <div className="metrics">
              <Metric label="历史" value={history.length} />
              <Metric label="完成" value={completed} tone="success" />
              <Metric label="失败" value={failed} tone="error" />
            </div>

            <LoginPanel
              status={loginStatus}
              checking={loginChecking}
              running={loginRunning}
              qr={loginQr}
              logs={loginLogs}
              desktopPath={desktopPath}
              desktopPasscode={desktopPasscode}
              disabled={busy || running}
              onDesktopPathChange={setDesktopPath}
              onDesktopPasscodeChange={setDesktopPasscode}
              onRefresh={() => void refreshLoginStatus()}
              onDesktopLogin={() => void startLogin("desktop")}
              onQrLogin={() => void startLogin("qr")}
              onCancel={() => void cancelLogin()}
            />

            <div className="activity-panel">
              <div className="section-header">
                <h2>当前输出</h2>
                <span>{progressSummary}</span>
              </div>
              {fileProgresses.length ? (
                <FileProgressList items={fileProgresses} />
              ) : (
                <div className={`progress-track ${running && progress === null ? "indeterminate" : ""}`}>
                  <div className="progress-fill" style={{ width: `${progressValue}%` }} />
                </div>
              )}
              <div className="output-summary">
                <span>{latestLog}</span>
                <button className="text-button" onClick={() => setLogsOpen((value) => !value)}>
                  {logsOpen ? "收起日志" : "查看日志"}
                </button>
              </div>
              {logsOpen ? (
                <div className="log-pane">
                  {logs.length ? (
                    logs.map((line, index) => <p key={`${line}-${index}`}>{line}</p>)
                  ) : (
                    <p>暂无日志</p>
                  )}
                </div>
              ) : null}
            </div>
          </section>
        </div>
      </section>

      <section className="history-section">
        <div className="section-header">
          <h2>任务历史</h2>
          <button className="ghost-button" onClick={clearHistory}>
            <Trash2 size={16} />
            清空
          </button>
        </div>

        <div className="history-list">
          {history.length ? (
            history.map((record) => <HistoryItem key={record.id} record={record} />)
          ) : (
            <div className="empty-state">
              <ListChecks size={20} />
              <span>暂无记录</span>
            </div>
          )}
        </div>
      </section>
    </main>
  );
}

function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className="toggle">
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      <span />
      {label}
    </label>
  );
}

function Metric({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone?: "success" | "error";
}) {
  return (
    <div className={`metric ${tone ?? ""}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function FileProgressList({ items }: { items: DownloadFileProgress[] }) {
  return (
    <div className="file-progress-list">
      {items.map((item) => (
        <div className="file-progress-item" key={item.key}>
          <div className="file-progress-top">
            <span>{item.name}</span>
            <strong>{Math.round(item.progress)}%</strong>
          </div>
          <div className="progress-track">
            <div className="progress-fill" style={{ width: `${Math.max(0, Math.min(100, item.progress))}%` }} />
          </div>
        </div>
      ))}
    </div>
  );
}

function LoginPanel({
  status,
  checking,
  running,
  qr,
  logs,
  desktopPath,
  desktopPasscode,
  disabled,
  onDesktopPathChange,
  onDesktopPasscodeChange,
  onRefresh,
  onDesktopLogin,
  onQrLogin,
  onCancel,
}: {
  status: LoginStatus;
  checking: boolean;
  running: boolean;
  qr: string;
  logs: string[];
  desktopPath: string;
  desktopPasscode: string;
  disabled: boolean;
  onDesktopPathChange: (value: string) => void;
  onDesktopPasscodeChange: (value: string) => void;
  onRefresh: () => void;
  onDesktopLogin: () => void;
  onQrLogin: () => void;
  onCancel: () => void;
}) {
  const StatusIcon = status.loggedIn ? ShieldCheck : running || checking ? LoaderCircle : AlertCircle;

  return (
    <div className="login-panel">
      <div className="section-header">
        <h2>Telegram 登录</h2>
        <button className="text-button" onClick={onRefresh} disabled={checking || running}>
          {checking ? "检查中" : "检查"}
        </button>
      </div>

      <div className={`login-state ${status.loggedIn ? "ready" : "warning"} ${checking || running ? "loading" : ""}`}>
        <StatusIcon size={17} />
        <span>{status.message}</span>
      </div>
      {status.detail ? <p className="login-detail">{status.detail}</p> : null}

      <div className="login-fields">
        <label className="field compact">
          <span>Desktop 数据目录</span>
          <input
            value={desktopPath}
            onChange={(event) => onDesktopPathChange(event.target.value)}
            placeholder="留空自动查找"
          />
        </label>
        <label className="field compact">
          <span>Desktop 本地密码</span>
          <input
            type="password"
            value={desktopPasscode}
            onChange={(event) => onDesktopPasscodeChange(event.target.value)}
            placeholder="无密码可留空"
          />
        </label>
      </div>

      <div className="login-actions">
        <button className="ghost-button" onClick={onDesktopLogin} disabled={disabled || running || checking}>
          <LogIn size={16} />
          连接 Desktop
        </button>
        <button className="ghost-button" onClick={onQrLogin} disabled={disabled || running || checking}>
          <QrCode size={16} />
          QR 登录
        </button>
        <button className="danger-button" onClick={onCancel} disabled={!running}>
          <Square size={16} />
          取消
        </button>
      </div>

      {qr ? <pre className="qr-box">{qr}</pre> : null}
      {logs.length ? (
        <div className="login-log">
          {logs.map((line, index) => (
            <p key={`${line}-${index}`}>{line}</p>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function ChatBrowser({
  chats,
  selectedChat,
  messages,
  selectedMessageIds,
  chatSearch,
  messageCount,
  loadingChats,
  loadingMessages,
  onSearchChange,
  onMessageCountChange,
  onLoadChats,
  onSelectChat,
  onToggleMessage,
  onToggleAll,
}: {
  chats: ChatInfo[];
  selectedChat: ChatInfo | null;
  messages: MessageInfo[];
  selectedMessageIds: Set<number>;
  chatSearch: string;
  messageCount: number;
  loadingChats: boolean;
  loadingMessages: boolean;
  onSearchChange: (value: string) => void;
  onMessageCountChange: (value: number) => void;
  onLoadChats: () => void;
  onSelectChat: (chat: ChatInfo) => void;
  onToggleMessage: (id: number) => void;
  onToggleAll: () => void;
}) {
  return (
    <div className="chat-browser">
      <div className="chat-toolbar">
        <label className="field compact">
          <span>搜索对话</span>
          <input
            value={chatSearch}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder="名称、用户名、ID"
          />
        </label>
        <label className="field compact count-field">
          <span>最近消息</span>
          <input
            type="number"
            min={1}
            max={500}
            value={messageCount}
            onChange={(event) => onMessageCountChange(Number(event.target.value))}
          />
        </label>
        <button className="ghost-button" onClick={onLoadChats} disabled={loadingChats}>
          {loadingChats ? <LoaderCircle size={16} /> : <RefreshCcw size={16} />}
          加载对话
        </button>
      </div>

      <div className="chat-grid">
        <div className="chat-list">
          {chats.length ? (
            chats.map((chat) => (
              <button
                className={`chat-item ${selectedChat?.id === chat.id ? "active" : ""}`}
                key={chat.id}
                onClick={() => onSelectChat(chat)}
                disabled={loadingMessages}
              >
                <strong>{chat.name}</strong>
                <span>{chat.chatType || "chat"} · {chat.username ? `@${chat.username}` : chat.id}</span>
              </button>
            ))
          ) : (
            <div className="chat-empty">点击“加载对话”读取 Telegram 对话列表</div>
          )}
        </div>

        <div className="message-list">
          <div className="message-list-header">
            <strong>{selectedChat ? selectedChat.name : "消息"}</strong>
            <button className="text-button" onClick={onToggleAll} disabled={!messages.length || loadingMessages}>
              {selectedMessageIds.size === messages.length && messages.length ? "取消全选" : "全选"}
            </button>
          </div>
          {loadingMessages ? (
            <div className="chat-empty"><LoaderCircle size={16} /> 正在读取消息...</div>
          ) : messages.length ? (
            messages.map((item) => (
              <label className="message-item" key={item.id}>
                <input
                  type="checkbox"
                  checked={selectedMessageIds.has(item.id)}
                  onChange={() => onToggleMessage(item.id)}
                />
                <div>
                  <div className="message-top">
                    <strong>#{item.id}</strong>
                    <span>{formatDate(item.date)}</span>
                  </div>
                  <p>{item.text || item.fileName || item.mediaType || "无文字内容"}</p>
                  <div className="message-meta">
                    {item.fileName ? <span>{item.fileName}</span> : null}
                    {item.fileSize ? <span>{formatFileSize(item.fileSize)}</span> : null}
                    {item.mediaType ? <span>{item.mediaType}</span> : null}
                  </div>
                </div>
              </label>
            ))
          ) : (
            <div className="chat-empty">选择一个对话后读取最近消息</div>
          )}
        </div>
      </div>
    </div>
  );
}

function LinkPreviewPanel({ state }: { state: PreviewState }) {
  if (state.status === "idle") {
    return null;
  }

  if (state.status === "loading") {
    return (
      <div className="link-preview-panel">
        <div className="link-preview-icon loading">
          <LoaderCircle size={17} />
        </div>
        <div className="link-preview-main">
          <div className="link-preview-title">
            <strong>正在读取消息文字</strong>
            <span>{state.link}</span>
          </div>
          <p>需要已登录 tdl，预览失败不影响下载。</p>
        </div>
      </div>
    );
  }

  if (state.status === "error") {
    return (
      <div className="link-preview-panel warning">
        <div className="link-preview-icon warning">
          <AlertCircle size={17} />
        </div>
        <div className="link-preview-main">
          <div className="link-preview-title">
            <strong>无法读取消息预览</strong>
            <span>{state.link}</span>
          </div>
          <p>{state.error}</p>
        </div>
      </div>
    );
  }

  const { preview } = state;
  const text = preview.text?.trim() || "这条消息没有可识别的文字内容。";

  return (
    <div className="link-preview-panel">
      <div className="link-preview-icon">
        <MessageSquareText size={17} />
      </div>
      <div className="link-preview-main">
        <div className="link-preview-title">
          <strong>@{preview.chat}/{preview.messageId}</strong>
          <span>{preview.mediaCount > 0 ? `媒体字段 ${preview.mediaCount}` : "文字预览"}</span>
        </div>
        <p>{text}</p>
      </div>
    </div>
  );
}

function HistoryItem({ record }: { record: DownloadRecord }) {
  const Icon =
    record.status === "completed"
      ? CheckCircle2
      : record.status === "failed"
        ? XCircle
        : record.status === "cancelled"
          ? AlertCircle
          : Download;

  return (
    <article className="history-item">
      <div className={`history-icon ${record.status}`}>
        <Icon size={18} />
      </div>
      <div className="history-main">
        <div className="history-top">
          <strong>{record.source}</strong>
          <span className={`pill ${record.status}`}>{statusLabel[record.status]}</span>
        </div>
        <div className="history-meta">
          <span>{modeLabel[record.mode]}</span>
          <span>{record.directory}</span>
          <span>{formatDate(record.completedAt ?? record.createdAt)}</span>
        </div>
        {record.error ? <p>{record.error}</p> : null}
      </div>
    </article>
  );
}

export default App;
