import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AlertCircle,
  Bot,
  CheckCircle2,
  CheckSquare2,
  ChevronDown,
  Download,
  Eye,
  File,
  FileJson,
  FolderOpen,
  Image as ImageIcon,
  Link as LinkIcon,
  ListChecks,
  LogIn,
  LoaderCircle,
  MessageSquareText,
  Music,
  Play,
  Search,
  QrCode,
  RefreshCcw,
  RotateCw,
  Settings2,
  ShieldCheck,
  Square,
  Terminal,
  Trash2,
  Video,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  AppConfig,
  AppState,
  ChatDownloadRequest,
  ChatInfo,
  ChatMediaPreview,
  ChatMediaPreviewFile,
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
  MediaKind,
  LinkPreview,
  SourceMode,
  TdlInfo,
  TdlUpdateEvent,
  TgLiteChat,
  TgLiteEvent,
  TgLiteStatus,
} from "./types";

const DEFAULT_CONFIG: AppConfig = {
  lastDirectory: "",
  limit: 4,
  threads: 4,
  pool: 8,
  tdlOverridePath: null,
  tgLiteApiId: "",
  tgLiteApiHash: "",
};

const statusLabel: Record<DownloadStatus, string> = {
  downloading: "下载中",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

const modeLabel: Record<SourceMode | "history", string> = {
  links: "链接",
  json: "JSON",
  raw: "原始参数",
  chat: "对话",
  tgLite: "TG Lite",
  history: "任务历史",
};

const AUTO_CHAT_REFRESH_MS = 60_000;
const AUTO_PREVIEW_LIMIT = 4;
const AUTO_PREVIEW_CONCURRENCY = 1;
const AUTO_PREVIEW_MAX_PHOTO_BYTES = 20 * 1024 * 1024;
const AUTO_PREVIEW_MAX_VIDEO_BYTES = 30 * 1024 * 1024;

type PreviewState =
  | { status: "idle" }
  | { status: "loading"; link: string }
  | { status: "ready"; link: string; preview: LinkPreview }
  | { status: "error"; link: string; error: string };

type MediaPreviewState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; preview: ChatMediaPreview }
  | { status: "skipped"; reason: "tooLarge" | "videoTooLarge" | "unsupported" }
  | { status: "error"; error: string };

type QueuedPreview = {
  chatId: string;
  generation: number;
  item: MessageInfo;
};

type AppMode = SourceMode | "history";

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

function mediaKindLabel(kind: MediaKind) {
  const labels: Record<MediaKind, string> = {
    none: "消息",
    photo: "图片",
    video: "视频",
    audio: "音频",
    document: "文件",
    unknown: "媒体",
  };
  return labels[kind] ?? "媒体";
}

function canPreviewMedia(kind: MediaKind) {
  return kind === "photo" || kind === "video";
}

function shouldAutoDownloadPreview(message: MessageInfo) {
  if (!message.previewable || !canPreviewMedia(message.mediaKind)) return false;
  if (message.mediaKind === "photo") {
    return !message.fileSize || message.fileSize <= AUTO_PREVIEW_MAX_PHOTO_BYTES;
  }
  if (message.mediaKind === "video") {
    return Boolean(message.fileSize && message.fileSize <= AUTO_PREVIEW_MAX_VIDEO_BYTES);
  }
  return false;
}

function skippedPreviewState(message: MessageInfo): MediaPreviewState {
  if (message.mediaKind === "video") return { status: "skipped", reason: "videoTooLarge" };
  if (message.mediaKind === "photo") return { status: "skipped", reason: "tooLarge" };
  return { status: "skipped", reason: "unsupported" };
}

function mediaIcon(kind: MediaKind) {
  if (kind === "photo") return ImageIcon;
  if (kind === "video") return Video;
  if (kind === "audio") return Music;
  if (kind === "document") return File;
  return MessageSquareText;
}

function previewButtonLabel(message: MessageInfo, state: MediaPreviewState) {
  if (state.status === "loading") return "加载中";
  if (message.mediaKind === "video") {
    return state.status === "ready" ? "刷新预览" : "加载预览";
  }
  return state.status === "ready" ? "刷新缩略图" : "加载缩略图";
}

function loggedInLabel(status: LoginStatus) {
  if (status.username) return `已登录：@${status.username}`;
  if (status.displayName) return `已登录：${status.displayName}`;
  return status.message;
}

function loggedOutStatus(message: string, detail?: string | null): LoginStatus {
  return {
    loggedIn: false,
    message,
    detail: detail ?? null,
    username: null,
    displayName: null,
  };
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
  if (request.mode === "tgLite") {
    return "TG Lite 复用 tdl 登录态浏览消息，下载时交给 tdl";
  }

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
  const [mode, setMode] = useState<AppMode>("links");
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
  const [loginStatus, setLoginStatus] = useState<LoginStatus>(
    loggedOutStatus("尚未检查登录状态"),
  );
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
  const [mediaPreviews, setMediaPreviews] = useState<Record<number, MediaPreviewState>>({});
  const [message, setMessage] = useState("正在加载");
  const [busy, setBusy] = useState(false);
  const [tdlUpdateChecking, setTdlUpdateChecking] = useState(false);

  const configSaveTimer = useRef<number | null>(null);
  const pendingConfigRef = useRef<AppConfig | null>(null);
  const previewRequestSeq = useRef(0);
  const selectedChatRef = useRef<ChatInfo | null>(null);
  const mediaPreviewsRef = useRef<Record<number, MediaPreviewState>>({});
  const previewQueueRef = useRef<QueuedPreview[]>([]);
  const queuedPreviewIdsRef = useRef<Set<number>>(new Set());
  const checkedPreviewCacheIdsRef = useRef<Set<number>>(new Set());
  const previewRunningRef = useRef(0);
  const previewGenerationRef = useRef(0);
  const chatRefreshRunningRef = useRef(false);

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
    selectedChatRef.current = selectedChat;
  }, [selectedChat]);

  useEffect(() => {
    mediaPreviewsRef.current = mediaPreviews;
  }, [mediaPreviews]);

  useEffect(() => {
    void loadState();

    if (!inTauri()) return;

    let cancelled = false;
    let unlistenDownload: (() => void) | undefined;
    let unlistenLogin: (() => void) | undefined;
    let unlistenTdlUpdate: (() => void) | undefined;

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

    listen<TdlUpdateEvent>("tdl-update-event", (event) => {
      handleTdlUpdateEvent(event.payload);
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlistenTdlUpdate = fn;
        }
      })
      .catch((error) => {
        if (!cancelled) {
          console.error("订阅 tdl-update-event 失败", error);
        }
      });

    return () => {
      cancelled = true;
      unlistenDownload?.();
      unlistenLogin?.();
      unlistenTdlUpdate?.();
    };
  }, []);

  useEffect(() => {
    if (!config.lastDirectory) return;
    setDirectory((current) => current || config.lastDirectory);
  }, [config.lastDirectory]);

  useEffect(() => {
    if (!inTauri() || !loginStatus.loggedIn) return;
    const timer = window.setInterval(() => {
      void loadChats({ silent: true });
    }, AUTO_CHAT_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [loginStatus.loggedIn]);

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

  const downloadMode: SourceMode = mode === "history" ? "links" : mode;
  const request = useMemo<DownloadRequest>(
    () => ({
      mode: downloadMode,
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
      downloadMode,
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
        setLoginStatus(loggedOutStatus("tdl 不可用，无法检查 Telegram 登录状态。"));
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
      setLoginStatus(loggedOutStatus("无法检查 Telegram 登录状态。", String(error)));
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
      setLoginQr("");
      if (completed) {
        setLoginLogs([]);
        setLoginStatus(loggedOutStatus(event.message ?? "登录完成"));
        void refreshLoginStatus();
        void loadChats({ silent: true });
      } else {
        setLoginStatus(loggedOutStatus(event.message ?? "登录失败", event.error));
      }
      setMessage(event.message ?? "");
    }
  }

  function handleTdlUpdateEvent(event: TdlUpdateEvent) {
    setTdlUpdateChecking(false);
    if (event.tdl) {
      setTdl(event.tdl);
    }
    setMessage(event.message);
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

  async function loadChats(options?: { silent?: boolean }) {
    if (!inTauri() || chatRefreshRunningRef.current) return;
    chatRefreshRunningRef.current = true;
    if (!options?.silent) {
      setChatLoading(true);
      setMessage("正在读取对话列表");
    }
    try {
      const items = await invoke<ChatInfo[]>("list_chats");
      setChats(items);
      if (!options?.silent) {
        setMessage(`已加载 ${items.length} 个对话`);
      }
    } catch (error) {
      if (!options?.silent) {
        setMessage(String(error));
      }
    } finally {
      chatRefreshRunningRef.current = false;
      if (!options?.silent) {
        setChatLoading(false);
      }
    }
  }

  async function loadMessages(chat: ChatInfo) {
    if (!inTauri()) return;
    setSelectedChat(chat);
    setMessages([]);
    setSelectedMessageIds(new Set());
    setMediaPreviews({});
    previewGenerationRef.current += 1;
    previewQueueRef.current = [];
    queuedPreviewIdsRef.current.clear();
    checkedPreviewCacheIdsRef.current.clear();
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

  const loadMediaPreview = useCallback(async (item: MessageInfo, options?: { force?: boolean }) => {
    const chat = selectedChatRef.current;
    if (!inTauri() || !chat || !canPreviewMedia(item.mediaKind)) return;
    const current = mediaPreviewsRef.current[item.id];
    if (current?.status === "loading") return;
    if (!options?.force && current && current.status !== "idle") return;

    const generation = previewGenerationRef.current;
    const chatId = String(chat.id);
    setMediaPreviews((current) => ({ ...current, [item.id]: { status: "loading" } }));
    try {
      const preview = await invoke<ChatMediaPreview>("preview_chat_media", {
        chatId,
        messageId: item.id,
      });
      if (previewGenerationRef.current !== generation || String(selectedChatRef.current?.id) !== chatId) {
        return;
      }
      setMediaPreviews((current) => ({
        ...current,
        [item.id]: { status: "ready", preview },
      }));
    } catch (error) {
      if (previewGenerationRef.current !== generation || String(selectedChatRef.current?.id) !== chatId) {
        return;
      }
      setMediaPreviews((current) => ({
        ...current,
        [item.id]: { status: "error", error: String(error) },
      }));
    }
  }, []);

  const loadCachedMediaPreview = useCallback(async (item: MessageInfo) => {
    const chat = selectedChatRef.current;
    if (!inTauri() || !chat || !canPreviewMedia(item.mediaKind)) return false;
    const current = mediaPreviewsRef.current[item.id];
    if (current?.status === "ready") return true;
    if (current?.status === "loading") return true;
    if (checkedPreviewCacheIdsRef.current.has(item.id)) return false;

    const generation = previewGenerationRef.current;
    const chatId = String(chat.id);
    try {
      const preview = await invoke<ChatMediaPreview | null>("cached_chat_media_preview", {
        chatId,
        messageId: item.id,
      });
      if (previewGenerationRef.current !== generation || String(selectedChatRef.current?.id) !== chatId) {
        return false;
      }
      if (!preview || preview.files.length === 0) {
        checkedPreviewCacheIdsRef.current.add(item.id);
        return false;
      }
      setMediaPreviews((current) => ({
        ...current,
        [item.id]: { status: "ready", preview },
      }));
      return true;
    } catch {
      if (previewGenerationRef.current === generation && String(selectedChatRef.current?.id) === chatId) {
        checkedPreviewCacheIdsRef.current.add(item.id);
      }
      return false;
    }
  }, []);

  const loadAutoMediaPreview = useCallback(async (entry: QueuedPreview) => {
    if (
      previewGenerationRef.current !== entry.generation ||
      String(selectedChatRef.current?.id) !== entry.chatId
    ) {
      return;
    }

    const cached = await loadCachedMediaPreview(entry.item);
    if (cached) return;

    if (!shouldAutoDownloadPreview(entry.item)) {
      if (
        previewGenerationRef.current === entry.generation &&
        String(selectedChatRef.current?.id) === entry.chatId
      ) {
        setMediaPreviews((current) => ({
          ...current,
          [entry.item.id]: skippedPreviewState(entry.item),
        }));
      }
      return;
    }

    await loadMediaPreview(entry.item);
  }, [loadCachedMediaPreview, loadMediaPreview]);

  const drainMediaPreviewQueue = useCallback(() => {
    while (previewRunningRef.current < AUTO_PREVIEW_CONCURRENCY && previewQueueRef.current.length > 0) {
      const next = previewQueueRef.current.shift();
      if (!next) break;
      queuedPreviewIdsRef.current.delete(next.item.id);
      if (
        previewGenerationRef.current !== next.generation ||
        String(selectedChatRef.current?.id) !== next.chatId
      ) {
        continue;
      }
      previewRunningRef.current += 1;
      void loadAutoMediaPreview(next).finally(() => {
        previewRunningRef.current = Math.max(0, previewRunningRef.current - 1);
        drainMediaPreviewQueue();
      });
    }
  }, [loadAutoMediaPreview]);

  const enqueueAutoMediaPreviews = useCallback((items: MessageInfo[]) => {
    const chat = selectedChatRef.current;
    if (!chat) return;
    const chatId = String(chat.id);
    const generation = previewGenerationRef.current;

    for (const item of items) {
      if (!canPreviewMedia(item.mediaKind) || !item.previewable) continue;
      const current = mediaPreviewsRef.current[item.id];
      if (current && current.status !== "idle") continue;
      if (queuedPreviewIdsRef.current.has(item.id)) continue;
      queuedPreviewIdsRef.current.add(item.id);
      previewQueueRef.current.push({ chatId, generation, item });
    }
    drainMediaPreviewQueue();
  }, [drainMediaPreviewQueue]);

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
    if (mode === "tgLite") {
      setMessage("TG Lite 下载入口会在对话浏览接入后启用");
      return;
    }
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

  async function checkTdlUpdate() {
    if (!inTauri() || tdlUpdateChecking) return;
    setTdlUpdateChecking(true);
    setMessage("正在后台检查 tdl 更新");
    try {
      await invoke("update_tdl");
    } catch (error) {
      setTdlUpdateChecking(false);
      setMessage(String(error));
    }
  }

  async function startLogin(method: LoginMethod) {
    if (!inTauri() || loginRunning) return;
    setLoginRunning(true);
    setLoginQr("");
    setLoginLogs([]);
    setLoginStatus(
      loggedOutStatus(method === "desktop" ? "正在连接 Telegram Desktop" : "正在生成 QR 登录码"),
    );

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
      setLoginStatus(loggedOutStatus("启动登录失败", String(error)));
      setMessage(String(error));
    }
  }

  async function cancelLogin() {
    if (!inTauri() || !loginRunning) return;
    await invoke("cancel_login");
    setLoginStatus(loggedOutStatus("正在取消登录"));
  }

  async function logout() {
    if (!inTauri() || loginRunning) return;
    setBusy(true);
    setMessage("正在退出登录");
    try {
      const status = await invoke<LoginStatus>("logout");
      setLoginStatus(status);
      setLoginQr("");
      setLoginLogs([]);
      setMessage(status.message);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
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
  const workspaceMode = mode === "chat" || mode === "tgLite";

  return (
    <main className="app-shell">
      <aside className="app-sidebar">
        <header className="sidebar-header">
          <h1>TDL Desktop</h1>
        </header>

        <nav className="nav-menu">
          <button className={`nav-item ${mode === "links" ? "active" : ""}`} onClick={() => setMode("links")}>
            <LinkIcon size={18} />
            链接下载
          </button>
          <button className={`nav-item ${mode === "json" ? "active" : ""}`} onClick={() => setMode("json")}>
            <FileJson size={18} />
            JSON 导入
          </button>
          <button className={`nav-item ${mode === "raw" ? "active" : ""}`} onClick={() => setMode("raw")}>
            <Terminal size={18} />
            原始参数
          </button>
          <button className={`nav-item ${mode === "chat" ? "active" : ""}`} onClick={() => setMode("chat")}>
            <Bot size={18} />
            对话浏览
          </button>
          <button className={`nav-item ${mode === "history" ? "active" : ""}`} onClick={() => setMode("history")}>
            <ListChecks size={18} />
            任务历史
          </button>
        </nav>
      </aside>

      <section className="main-workspace">
        <header className="topbar">
          <div className="status-line">
            <span className={`status-dot ${tdl?.available ? "ready" : "error"}`} />
            <span>{tdlSourceLabel(tdl)}</span>
            {tdl?.path ? <code>{tdl.path}</code> : null}
          </div>
          <div className="topbar-actions">
            <button className="ghost-button" onClick={loadState} disabled={busy}>
              <RefreshCcw size={16} />
              刷新
            </button>
            <button className="ghost-button" onClick={checkTdlUpdate} disabled={tdlUpdateChecking}>
              <RotateCw size={16} />
              {tdlUpdateChecking ? "检查中" : "检查更新"}
            </button>
          </div>
        </header>

        <div className="workspace-content">
          {mode === "chat" ? (
            <TelegramWorkspace
              chats={filteredChats}
              selectedChat={selectedChat}
              messages={messages}
              selectedMessageIds={selectedMessageIds}
              mediaPreviews={mediaPreviews}
              chatSearch={chatSearch}
              messageCount={messageCount}
              loadingChats={chatLoading}
              loadingMessages={messagesLoading}
              directory={directory}
              running={running}
              busy={busy}
              progressSummary={progressSummary}
              latestLog={latestLog}
              loginStatus={loginStatus}
              onSearchChange={setChatSearch}
              onMessageCountChange={setMessageCount}
              onLoadChats={() => void loadChats()}
              onSelectChat={(chat) => void loadMessages(chat)}
              onRefreshMessages={() => selectedChat && void loadMessages(selectedChat)}
              onAutoPreviewMessages={enqueueAutoMediaPreviews}
              onPreviewMessage={(message) => void loadMediaPreview(message, { force: true })}
              onToggleMessage={toggleMessage}
              onToggleAll={toggleAllMessages}
              onPickDirectory={() => void pickDirectory()}
              onDirectoryChange={setDirectory}
              onStartDownload={() => void startDownload()}
              onCancelDownload={() => void cancelDownload()}
            />
          ) : mode === "tgLite" ? (
            <TgLiteWorkspace
              config={config}
              onSaveConfig={(next) => void saveConfig(next)}
            />
          ) : mode === "history" ? (
            <section className="history-section" style={{ border: "none", boxShadow: "none", borderRadius: 0, padding: "32px" }}>
              <div className="section-header">
                <h2>下载记录</h2>
                <button className="ghost-button" onClick={clearHistory}>
                  <Trash2 size={16} />
                  清空历史
                </button>
              </div>
              <div className="history-list">
                {history.length ? (
                  history.map((record) => <HistoryItem key={record.id} record={record} />)
                ) : (
                  <div className="empty-state">
                    <ListChecks size={20} />
                    <span>暂无下载记录</span>
                  </div>
                )}
              </div>
            </section>
          ) : (
            <div className="content-grid">
              <section className="task-panel">
                <div className="section-header">
                  <h2>{modeLabel[mode]} 下载</h2>
                  <span>{message}</span>
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

                {mode !== "raw" ? (
                  <div className="directory-row">
                    <label className="field compact" style={{ flex: 1 }}>
                      <span>下载目录</span>
                      <input value={directory} onChange={(event) => setDirectory(event.target.value)} />
                    </label>
                    <button className="icon-button" onClick={pickDirectory} title="选择目录" style={{ marginTop: "24px" }}>
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
                  高级参数设置
                  <ChevronDown size={16} className={advancedOpen ? "open" : ""} />
                </button>

                {advancedOpen ? (
                  <div className="advanced-grid">
                    <Toggle label="群组模式" checked={group} onChange={setGroup} />
                    <Toggle label="跳过同名同大小" checked={skipSame} onChange={setSkipSame} />
                    <Toggle label="断点续传" checked={continueLast} onChange={setContinueLast} />
                    <Toggle label="重启任务" checked={restart} onChange={setRestart} />
                    <Toggle label="倒序导出" checked={desc} onChange={setDesc} />
                    <Toggle label="使用 Takeout" checked={takeout} onChange={setTakeout} />
                    <Toggle label="自动重写扩展名" checked={rewriteExt} onChange={setRewriteExt} />
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
                  <Terminal size={18} />
                  <code>{commandPreview}</code>
                </div>

                <div className="action-row">
                  <button className="primary-button" onClick={startDownload} disabled={busy || running}>
                    <Play size={17} />
                    开始任务
                  </button>
                  <button className="danger-button" onClick={cancelDownload} disabled={!running}>
                    <Square size={16} />
                    停止
                  </button>
                </div>
              </section>

              <section className="side-panel">
                <div className="metrics">
                  <Metric label="累计任务" value={history.length} />
                  <Metric label="成功" value={completed} tone="success" />
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
                  onLogout={() => void logout()}
                />

                <div className="activity-panel">
                  <div className="section-header">
                    <h2>实时进度</h2>
                    <span>{progressSummary}</span>
                  </div>
                  {fileProgresses.length ? (
                    <FileProgressList items={fileProgresses} />
                  ) : (
                    <div className={`progress-track ${running && progress === null ? "indeterminate" : ""}`}>
                      <div className="progress-fill" style={{ width: `${progressValue}%` }} />
                    </div>
                  )}
                  <div className="output-summary" style={{ marginTop: "12px" }}>
                    <span style={{ fontSize: "12px", color: "var(--text-secondary)" }}>{latestLog}</span>
                    {logs.length ? (
                      <button className="text-button" type="button" onClick={() => setLogsOpen((current) => !current)}>
                        {logsOpen ? "收起日志" : "查看日志"}
                      </button>
                    ) : null}
                  </div>
                  {logsOpen && logs.length ? (
                    <div className="log-pane" style={{ marginTop: "12px" }}>
                      {logs.map((line, index) => (
                        <p key={`${index}-${line}`}>{line}</p>
                      ))}
                    </div>
                  ) : null}
                </div>
              </section>
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
  onLogout,
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
  onLogout: () => void;
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
        <span>{status.loggedIn ? loggedInLabel(status) : status.message}</span>
      </div>
      {status.detail ? <p className="login-detail">{status.detail}</p> : null}

      {!status.loggedIn || running ? (
        <>
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
        </>
      ) : (
        <div className="login-actions">
          <button className="danger-button" onClick={onLogout} disabled={disabled || checking}>
            <LogIn size={16} />
            退出登录
          </button>
        </div>
      )}

      {!status.loggedIn && qr ? <pre className="qr-box">{qr}</pre> : null}
      {!status.loggedIn && logs.length ? (
        <div className="login-log">
          {logs.map((line, index) => (
            <p key={`${line}-${index}`}>{line}</p>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function TgLiteWorkspace({
  config,
  onSaveConfig,
}: {
  config: AppConfig;
  onSaveConfig: (config: AppConfig) => void;
}) {
  const [status, setStatus] = useState<TgLiteStatus | null>(null);
  const [connectionState, setConnectionState] = useState("");
  const [query, setQuery] = useState("");
  const [chats, setChats] = useState<TgLiteChat[]>([]);
  const [selectedChat, setSelectedChat] = useState<TgLiteChat | null>(null);
  const [messages, setMessages] = useState<MessageInfo[]>([]);
  const [messageCount, setMessageCount] = useState(50);
  const [loadingStatus, setLoadingStatus] = useState(false);
  const [loadingChats, setLoadingChats] = useState(false);
  const [loadingMessages, setLoadingMessages] = useState(false);
  const [apiId, setApiId] = useState(config.tgLiteApiId ?? "");
  const [apiHash, setApiHash] = useState(config.tgLiteApiHash ?? "");
  const [phoneNumber, setPhoneNumber] = useState("");
  const [authCode, setAuthCode] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const selectedChatRef = useRef<TgLiteChat | null>(null);
  const loadingChatsRef = useRef(false);

  const effectiveStatus: TgLiteStatus =
    status ?? {
      configured: Boolean(config.tgLiteApiId?.trim() && config.tgLiteApiHash?.trim()),
      initialized: false,
      authorized: false,
      state: "unknown",
      message: "TG Lite 使用独立 TDLib 只读会话。",
      qrLink: null,
      username: null,
      displayName: null,
    };

  const filteredChats = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    const sorted = sortTgLiteChats(chats);
    if (!keyword) return sorted;
    return sorted.filter((chat) =>
      [chat.title, chat.chatType, String(chat.id), chat.lastMessageText ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(keyword),
    );
  }, [chats, query]);

  useEffect(() => {
    setApiId(config.tgLiteApiId ?? "");
    setApiHash(config.tgLiteApiHash ?? "");
  }, [config.tgLiteApiHash, config.tgLiteApiId]);

  useEffect(() => {
    selectedChatRef.current = selectedChat;
  }, [selectedChat]);

  useEffect(() => {
    loadingChatsRef.current = loadingChats;
  }, [loadingChats]);

  useEffect(() => {
    if (!inTauri()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    listen<TgLiteEvent>("tg-lite-event", (event) => {
      if (!cancelled) {
        handleTgLiteEvent(event.payload);
      }
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((error) => {
        if (!cancelled) setError(String(error));
      });

    void refreshStatus({ start: true });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (status?.authorized && chats.length === 0 && !loadingChats) {
      void loadTgLiteChats();
    }
  }, [chats.length, loadingChats, status?.authorized]);

  function handleTgLiteEvent(event: TgLiteEvent) {
    if (event.kind === "status") {
      setStatus(event.status);
      setError("");
      return;
    }

    if (event.kind === "connection") {
      setConnectionState(event.state);
      return;
    }

    if (event.kind === "chatUpsert") {
      setChats((current) => upsertTgLiteChat(current, event.chat));
      setSelectedChat((current) => current?.id === event.chat.id ? { ...current, ...event.chat } : current);
      return;
    }

    if (event.kind === "chatDelete") {
      setChats((current) => current.filter((chat) => chat.id !== event.chatId));
      setSelectedChat((current) => current?.id === event.chatId ? null : current);
      return;
    }

    if (event.kind === "messageNew") {
      setChats((current) => current.map((chat) => chat.id === event.chatId ? {
        ...chat,
        lastMessageId: event.message.id,
        lastMessageText: event.message.text ?? chat.lastMessageText,
      } : chat));
      if (selectedChatRef.current?.id === event.chatId) {
        setMessages((current) => sortMessages(upsertMessage(current, event.message)));
      }
      return;
    }

    if (event.kind === "messageUpdate") {
      if (selectedChatRef.current?.id === event.chatId && event.message) {
        setMessages((current) => sortMessages(upsertMessage(current, event.message!)));
      }
      return;
    }

    if (event.kind === "messageDelete" && selectedChatRef.current?.id === event.chatId) {
      const deleted = new Set(event.messageIds);
      setMessages((current) => current.filter((message) => !deleted.has(message.id)));
    }
  }

  async function refreshStatus(options?: { start?: boolean }) {
    if (!inTauri()) return;
    setLoadingStatus(true);
    setError("");
    try {
      const next = options?.start
        ? await invoke<TgLiteStatus>("tg_lite_start")
        : await invoke<TgLiteStatus>("tg_lite_status");
      setStatus(next);
    } catch (error) {
      setError(String(error));
      try {
        setStatus(await invoke<TgLiteStatus>("tg_lite_status"));
      } catch {
        // keep the original error visible
      }
    } finally {
      setLoadingStatus(false);
    }
  }

  async function saveTgLiteConfig() {
    const next = { ...config, tgLiteApiId: apiId.trim(), tgLiteApiHash: apiHash.trim() };
    onSaveConfig(next);
    setStatus((current) => current ? { ...current, configured: Boolean(next.tgLiteApiId && next.tgLiteApiHash) } : current);
    window.setTimeout(() => void refreshStatus({ start: true }), 150);
  }

  async function requestQrLogin() {
    setLoadingStatus(true);
    setError("");
    try {
      setStatus(await invoke<TgLiteStatus>("tg_lite_request_qr"));
    } catch (error) {
      setError(String(error));
    } finally {
      setLoadingStatus(false);
    }
  }

  async function submitPhone() {
    if (!phoneNumber.trim()) return;
    setLoadingStatus(true);
    setError("");
    try {
      setStatus(await invoke<TgLiteStatus>("tg_lite_set_phone", { phoneNumber: phoneNumber.trim() }));
    } catch (error) {
      setError(String(error));
    } finally {
      setLoadingStatus(false);
    }
  }

  async function submitCode() {
    if (!authCode.trim()) return;
    setLoadingStatus(true);
    setError("");
    try {
      setStatus(await invoke<TgLiteStatus>("tg_lite_check_code", { code: authCode.trim() }));
      setAuthCode("");
    } catch (error) {
      setError(String(error));
    } finally {
      setLoadingStatus(false);
    }
  }

  async function submitPassword() {
    if (!password) return;
    setLoadingStatus(true);
    setError("");
    try {
      setStatus(await invoke<TgLiteStatus>("tg_lite_check_password", { password }));
      setPassword("");
    } catch (error) {
      setError(String(error));
    } finally {
      setLoadingStatus(false);
    }
  }

  async function loadTgLiteChats() {
    if (!inTauri() || loadingChatsRef.current) return;
    loadingChatsRef.current = true;
    setLoadingChats(true);
    setError("");
    try {
      const list = await invoke<TgLiteChat[]>("tg_lite_load_chats", { limit: 100 });
      setChats(sortTgLiteChats(list));
      if (list.length && !selectedChatRef.current) {
        await selectTgLiteChat(sortTgLiteChats(list)[0]);
      }
    } catch (error) {
      setError(String(error));
      await refreshStatus();
    } finally {
      loadingChatsRef.current = false;
      setLoadingChats(false);
    }
  }

  async function selectTgLiteChat(chat: TgLiteChat) {
    setSelectedChat(chat);
    setMessages([]);
    if (!inTauri()) return;
    setLoadingMessages(true);
    setError("");
    try {
      const list = await invoke<MessageInfo[]>("tg_lite_load_messages", {
        chatId: chat.id,
        limit: messageCount,
      });
      setMessages(sortMessages(list));
    } catch (error) {
      setError(String(error));
      await refreshStatus();
    } finally {
      setLoadingMessages(false);
    }
  }

  async function refreshTgLiteMessages() {
    if (selectedChat) {
      await selectTgLiteChat(selectedChat);
    }
  }

  const statusReady = effectiveStatus.authorized;
  const accountLabel = effectiveStatus.username
    ? `@${effectiveStatus.username}`
    : effectiveStatus.displayName || effectiveStatus.message;
  const needsBootstrap = !statusReady;

  return (
    <div className="tg-lite-workspace">
      <aside className="tg-lite-dialogs">
        <div className="tg-lite-sidebar-head">
            <div>
              <strong>TG Lite</strong>
              <span>{statusReady ? accountLabel : effectiveStatus.message}</span>
            </div>
            <div className="tg-lite-sidebar-actions">
              <button className="icon-button compact-icon" type="button" onClick={() => void refreshStatus({ start: !statusReady })} disabled={loadingStatus} title="刷新状态">
                {loadingStatus ? <LoaderCircle size={17} /> : statusReady ? <ShieldCheck size={17} /> : <AlertCircle size={17} />}
              </button>
              <button className="icon-button compact-icon" type="button" onClick={loadTgLiteChats} disabled={!statusReady || loadingChats} title="刷新对话">
                {loadingChats ? <LoaderCircle size={17} /> : <RefreshCcw size={17} />}
              </button>
            </div>
          </div>

          {error ? <p className="tg-lite-sidebar-error">{error}</p> : null}

          <div className="telegram-search">
            <Search size={16} />
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索对话或消息预览" disabled={!statusReady} />
          </div>

          <div className="tg-lite-dialog-list">
            {!statusReady ? (
              <div className="telegram-empty">完成一次 TDLib 只读连接后，对话会在这里实时更新。</div>
            ) : loadingChats ? (
              <div className="telegram-empty">
                <LoaderCircle size={18} />
                正在加载对话
              </div>
            ) : filteredChats.length ? (
              filteredChats.map((chat) => (
                <button
                  className={`telegram-chat-item tg-lite-dialog-item ${selectedChat?.id === chat.id ? "active" : ""}`}
                  key={chat.id}
                  onClick={() => void selectTgLiteChat(chat)}
                  disabled={loadingMessages}
                >
                  <div className="chat-avatar">{chat.title.trim().slice(0, 1).toUpperCase() || "T"}</div>
                  <div className="telegram-chat-main">
                    <div className="tg-lite-dialog-title">
                      <strong>{chat.title}</strong>
                      {chat.unreadCount ? <span className="tg-lite-unread-badge">{chat.unreadCount}</span> : null}
                    </div>
                    <span className="tg-lite-dialog-preview">{chat.lastMessageText?.trim() || `${chat.chatType} · ${chat.id}`}</span>
                  </div>
                </button>
              ))
            ) : (
              <div className="telegram-empty">没有可显示的对话</div>
            )}
          </div>
        </aside>

        <section className="tg-lite-messages">
          <header className="tg-lite-message-header">
            <div className="telegram-title">
              <div className="chat-avatar large">{selectedChat?.title.trim().slice(0, 1).toUpperCase() || "T"}</div>
              <div>
                <strong>{selectedChat?.title ?? "TG Lite 只读浏览器"}</strong>
                <span>
                  {selectedChat
                    ? `${messages.length} 条消息 · ${selectedChat.chatType}`
                    : statusReady ? `实时接收中${connectionState ? ` · ${connectionState}` : ""}` : effectiveStatus.message}
                </span>
              </div>
            </div>
            <div className="telegram-actions tg-lite-reader-actions">
              <select
                value={messageCount}
                onChange={(event) => setMessageCount(Number(event.target.value))}
                disabled={loadingMessages || !statusReady}
              >
                <option value={50}>最近 50</option>
                <option value={100}>最近 100</option>
                <option value={200}>最近 200</option>
              </select>
              <button className="ghost-button" onClick={() => void refreshTgLiteMessages()} disabled={!selectedChat || loadingMessages}>
                {loadingMessages ? <LoaderCircle size={16} /> : <RefreshCcw size={16} />}
                刷新
              </button>
            </div>
          </header>

          <div className="tg-lite-message-list">
            {needsBootstrap ? (
              <TgLiteBootstrap
                status={effectiveStatus}
                loading={loadingStatus}
                apiId={apiId}
                apiHash={apiHash}
                phoneNumber={phoneNumber}
                authCode={authCode}
                password={password}
                onApiIdChange={setApiId}
                onApiHashChange={setApiHash}
                onPhoneNumberChange={setPhoneNumber}
                onAuthCodeChange={setAuthCode}
                onPasswordChange={setPassword}
                onSaveConfig={() => void saveTgLiteConfig()}
                onRequestQr={() => void requestQrLogin()}
                onSubmitPhone={() => void submitPhone()}
                onSubmitCode={() => void submitCode()}
                onSubmitPassword={() => void submitPassword()}
              />
            ) : !selectedChat ? (
              <div className="tg-lite-locked">
                <MessageSquareText size={38} />
                <strong>选择一个对话开始浏览</strong>
                <span>这里是只读阅读模式，没有发送框，也不会提供账号管理功能。</span>
              </div>
            ) : loadingMessages ? (
              <div className="tg-lite-locked">
                <LoaderCircle size={34} />
                <strong>正在读取消息</strong>
                <span>正在读取最近 {messageCount} 条消息。</span>
              </div>
            ) : messages.length ? (
              messages.map((item) => <TgLiteMessageBubble key={item.id} message={item} />)
            ) : (
              <div className="tg-lite-locked">
                <MessageSquareText size={38} />
                <strong>没有可显示的消息</strong>
                <span>可以刷新消息或增加最近消息数量。</span>
              </div>
            )}
          </div>
        </section>
    </div>
  );
}

function sortTgLiteChats(items: TgLiteChat[]) {
  return [...items].sort((left, right) => {
    const leftOrder = left.order ? BigInt(left.order) : 0n;
    const rightOrder = right.order ? BigInt(right.order) : 0n;
    if (leftOrder !== rightOrder) return rightOrder > leftOrder ? 1 : -1;
    return right.id - left.id;
  });
}

function upsertTgLiteChat(items: TgLiteChat[], chat: TgLiteChat) {
  const index = items.findIndex((item) => item.id === chat.id);
  const next = index === -1
    ? [...items, chat]
    : items.map((item, itemIndex) => itemIndex === index ? { ...item, ...chat } : item);
  return sortTgLiteChats(next);
}

function sortMessages(items: MessageInfo[]) {
  return [...items].sort((left, right) => left.id - right.id);
}

function upsertMessage(items: MessageInfo[], message: MessageInfo) {
  const index = items.findIndex((item) => item.id === message.id);
  if (index === -1) return [...items, message];
  return items.map((item, itemIndex) => itemIndex === index ? { ...item, ...message } : item);
}

function TgLiteBootstrap({
  status,
  loading,
  apiId,
  apiHash,
  phoneNumber,
  authCode,
  password,
  onApiIdChange,
  onApiHashChange,
  onPhoneNumberChange,
  onAuthCodeChange,
  onPasswordChange,
  onSaveConfig,
  onRequestQr,
  onSubmitPhone,
  onSubmitCode,
  onSubmitPassword,
}: {
  status: TgLiteStatus;
  loading: boolean;
  apiId: string;
  apiHash: string;
  phoneNumber: string;
  authCode: string;
  password: string;
  onApiIdChange: (value: string) => void;
  onApiHashChange: (value: string) => void;
  onPhoneNumberChange: (value: string) => void;
  onAuthCodeChange: (value: string) => void;
  onPasswordChange: (value: string) => void;
  onSaveConfig: () => void;
  onRequestQr: () => void;
  onSubmitPhone: () => void;
  onSubmitCode: () => void;
  onSubmitPassword: () => void;
}) {
  return (
    <div className="tg-lite-bootstrap">
      <ShieldCheck size={42} />
      <strong>连接 TG Lite 只读会话</strong>
      <span>{status.message}</span>

      {!status.configured ? (
        <div className="tg-lite-bootstrap-card">
          <label className="field compact">
            <span>api_id</span>
            <input value={apiId} onChange={(event) => onApiIdChange(event.target.value)} placeholder="Telegram API ID" />
          </label>
          <label className="field compact">
            <span>api_hash</span>
            <input value={apiHash} onChange={(event) => onApiHashChange(event.target.value)} placeholder="Telegram API Hash" />
          </label>
          <button className="primary-button" onClick={onSaveConfig} disabled={loading || !apiId.trim() || !apiHash.trim()}>
            {loading ? <LoaderCircle size={16} /> : <ShieldCheck size={16} />}
            保存并连接
          </button>
        </div>
      ) : (
        <div className="tg-lite-bootstrap-card">
          {status.qrLink ? <code className="tg-lite-qr-code">{status.qrLink}</code> : null}
          <button className="primary-button" onClick={onRequestQr} disabled={loading}>
            {loading ? <LoaderCircle size={16} /> : <QrCode size={16} />}
            获取 QR 登录链接
          </button>
          {status.state === "waitPhoneNumber" || status.state === "notStarted" || status.state === "initializing" ? (
            <div className="tg-lite-auth-row">
              <input value={phoneNumber} onChange={(event) => onPhoneNumberChange(event.target.value)} placeholder="手机号，例如 +8613800000000" />
              <button className="ghost-button" onClick={onSubmitPhone} disabled={loading || !phoneNumber.trim()}>提交手机号</button>
            </div>
          ) : null}
          {status.state === "waitCode" ? (
            <div className="tg-lite-auth-row">
              <input value={authCode} onChange={(event) => onAuthCodeChange(event.target.value)} placeholder="验证码" />
              <button className="ghost-button" onClick={onSubmitCode} disabled={loading || !authCode.trim()}>提交验证码</button>
            </div>
          ) : null}
          {status.state === "waitPassword" ? (
            <div className="tg-lite-auth-row">
              <input type="password" value={password} onChange={(event) => onPasswordChange(event.target.value)} placeholder="二步验证密码" />
              <button className="ghost-button" onClick={onSubmitPassword} disabled={loading || !password}>提交密码</button>
            </div>
          ) : null}
        </div>
      )}
    </div>
  );
}

function TgLiteMessageBubble({
  message,
}: {
  message: MessageInfo;
}) {
  const Icon = mediaIcon(message.mediaKind);
  const hasText = Boolean(message.text?.trim());
  const mediaLabel = message.fileName || message.mediaType || `${mediaKindLabel(message.mediaKind)}消息`;

  return (
    <article className="tg-lite-message-bubble">
      <div className="tg-lite-message-body">
        {message.mediaKind !== "none" ? (
          <div className="tg-lite-media-pill">
            <Icon size={16} />
            <span>{mediaLabel}</span>
            {message.fileSize ? <em>{formatFileSize(message.fileSize)}</em> : null}
          </div>
        ) : null}
        <p className={!hasText ? "muted" : ""}>
          {hasText ? message.text : message.mediaKind !== "none" ? `${mediaKindLabel(message.mediaKind)}消息` : "无文字内容"}
        </p>
        <div className="tg-lite-message-meta">
          <span>{formatDate(message.date)}</span>
          <span>#{message.id}</span>
          {message.mimeType ? <span>{message.mimeType}</span> : null}
          {message.width && message.height ? <span>{message.width}x{message.height}</span> : null}
          {message.duration ? <span>{Math.round(message.duration)}s</span> : null}
        </div>
      </div>
    </article>
  );
}

function TelegramWorkspace({
  chats,
  selectedChat,
  messages,
  selectedMessageIds,
  mediaPreviews,
  chatSearch,
  messageCount,
  loadingChats,
  loadingMessages,
  directory,
  running,
  busy,
  progressSummary,
  latestLog,
  loginStatus,
  onSearchChange,
  onMessageCountChange,
  onLoadChats,
  onSelectChat,
  onRefreshMessages,
  onAutoPreviewMessages,
  onPreviewMessage,
  onToggleMessage,
  onToggleAll,
  onPickDirectory,
  onDirectoryChange,
  onStartDownload,
  onCancelDownload,
}: {
  chats: ChatInfo[];
  selectedChat: ChatInfo | null;
  messages: MessageInfo[];
  selectedMessageIds: Set<number>;
  mediaPreviews: Record<number, MediaPreviewState>;
  chatSearch: string;
  messageCount: number;
  loadingChats: boolean;
  loadingMessages: boolean;
  directory: string;
  running: boolean;
  busy: boolean;
  progressSummary: string;
  latestLog: string;
  loginStatus: LoginStatus;
  onSearchChange: (value: string) => void;
  onMessageCountChange: (value: number) => void;
  onLoadChats: () => void;
  onSelectChat: (chat: ChatInfo) => void;
  onRefreshMessages: () => void;
  onAutoPreviewMessages: (messages: MessageInfo[]) => void;
  onPreviewMessage: (message: MessageInfo) => void;
  onToggleMessage: (id: number) => void;
  onToggleAll: () => void;
  onPickDirectory: () => void;
  onDirectoryChange: (value: string) => void;
  onStartDownload: () => void;
  onCancelDownload: () => void;
}) {
  const selectedCount = selectedMessageIds.size;
  const allSelected = messages.length > 0 && selectedCount === messages.length;
  const messageListRef = useRef<HTMLDivElement | null>(null);
  const messageNodeRefs = useRef<Map<number, HTMLElement>>(new Map());
  const [visibleMessageIds, setVisibleMessageIds] = useState<Set<number>>(new Set());

  const registerMessageNode = useCallback((id: number, node: HTMLElement | null) => {
    if (node) {
      messageNodeRefs.current.set(id, node);
    } else {
      messageNodeRefs.current.delete(id);
    }
  }, []);

  useEffect(() => {
    setVisibleMessageIds(new Set());
  }, [selectedChat?.id, messages]);

  useEffect(() => {
    const root = messageListRef.current;
    if (!root || !messages.length || typeof IntersectionObserver === "undefined") return;

    const validIds = new Set(messages.map((item) => item.id));
    for (const id of Array.from(messageNodeRefs.current.keys())) {
      if (!validIds.has(id)) messageNodeRefs.current.delete(id);
    }

    const observer = new IntersectionObserver(
      (entries) => {
        setVisibleMessageIds((current) => {
          const next = new Set(current);
          let changed = false;
          for (const entry of entries) {
            const id = Number((entry.target as HTMLElement).dataset.messageId);
            if (!Number.isFinite(id)) continue;
            if (entry.isIntersecting) {
              if (!next.has(id)) {
                next.add(id);
                changed = true;
              }
            } else if (next.delete(id)) {
              changed = true;
            }
          }
          return changed ? next : current;
        });
      },
      { root, rootMargin: "120px 0px", threshold: 0.15 },
    );

    messageNodeRefs.current.forEach((node) => observer.observe(node));
    return () => observer.disconnect();
  }, [messages]);

  useEffect(() => {
    const visibleMedia = messages
      .filter((item) => visibleMessageIds.has(item.id) && item.previewable && item.mediaKind !== "none")
      .sort((left, right) => {
        const leftKind = left.mediaKind === "photo" ? 0 : left.mediaKind === "video" ? 1 : 2;
        const rightKind = right.mediaKind === "photo" ? 0 : right.mediaKind === "video" ? 1 : 2;
        if (leftKind !== rightKind) return leftKind - rightKind;
        return (left.fileSize ?? 0) - (right.fileSize ?? 0);
      })
      .slice(0, AUTO_PREVIEW_LIMIT);
    if (visibleMedia.length > 0) {
      onAutoPreviewMessages(visibleMedia);
    }
  }, [messages, onAutoPreviewMessages, visibleMessageIds]);

  return (
    <div className="telegram-workspace">
      <aside className="telegram-sidebar">
        <div className="telegram-sidebar-head">
          <div>
            <strong>对话</strong>
            <span>{loginStatus.loggedIn ? loggedInLabel(loginStatus) : "请先登录 Telegram"}</span>
          </div>
          <button className="icon-button compact-icon" onClick={onLoadChats} disabled={loadingChats} title="刷新对话">
            {loadingChats ? <LoaderCircle size={17} /> : <RefreshCcw size={17} />}
          </button>
        </div>
        <div className="telegram-search">
          <Search size={16} />
          <input
            value={chatSearch}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder="搜索对话、用户名或 ID"
          />
        </div>
        <div className="telegram-chat-list">
          {chats.length ? (
            chats.map((chat) => (
              <button
                className={`telegram-chat-item ${selectedChat?.id === chat.id ? "active" : ""}`}
                key={chat.id}
                onClick={() => onSelectChat(chat)}
                disabled={loadingMessages}
              >
                <div className="chat-avatar">{chat.name.trim().slice(0, 1).toUpperCase() || "T"}</div>
                <div className="telegram-chat-main">
                  <strong>{chat.name}</strong>
                  <span>{chat.chatType || "chat"} · {chat.username ? `@${chat.username}` : chat.id}</span>
                </div>
              </button>
            ))
          ) : (
            <div className="telegram-empty">点击右上角刷新读取对话列表</div>
          )}
        </div>
      </aside>

      <section className="telegram-main">
        <header className="telegram-header">
          <div className="telegram-title">
            <div className="chat-avatar large">{selectedChat?.name.trim().slice(0, 1).toUpperCase() || "T"}</div>
            <div>
              <strong>{selectedChat?.name ?? "选择一个对话"}</strong>
              <span>
                {selectedChat
                  ? `${messages.length} 条已加载 · ${selectedChat.username ? `@${selectedChat.username}` : selectedChat.id}`
                  : "只读浏览，选择消息后下载"}
              </span>
            </div>
          </div>
          <div className="telegram-actions">
            <select
              value={messageCount}
              onChange={(event) => onMessageCountChange(Number(event.target.value))}
              disabled={loadingMessages}
              title="最近消息数量"
            >
              <option value={50}>最近 50</option>
              <option value={100}>最近 100</option>
              <option value={200}>最近 200</option>
            </select>
            <button className="ghost-button" onClick={onRefreshMessages} disabled={!selectedChat || loadingMessages}>
              {loadingMessages ? <LoaderCircle size={16} /> : <RefreshCcw size={16} />}
              刷新消息
            </button>
            <button className="ghost-button" onClick={onToggleAll} disabled={!messages.length || loadingMessages}>
              <CheckSquare2 size={16} />
              {allSelected ? "取消全选" : "全选"}
            </button>
            <button className="primary-button" onClick={onStartDownload} disabled={!selectedChat || selectedCount === 0 || busy || running}>
              <Download size={17} />
              {selectedCount ? `下载选中 ${selectedCount} 条` : "下载"}
            </button>
            <button className="danger-button" onClick={onCancelDownload} disabled={!running}>
              <Square size={16} />
              取消
            </button>
          </div>
        </header>

        <div className="telegram-download-bar">
          <label className="field compact">
            <span>下载目录</span>
            <input value={directory} onChange={(event) => onDirectoryChange(event.target.value)} />
          </label>
          <button className="icon-button compact-icon" onClick={onPickDirectory} title="选择目录">
            <FolderOpen size={18} />
          </button>
          <div className="telegram-run-state">
            <strong>{progressSummary}</strong>
            <span>{latestLog}</span>
          </div>
        </div>

        <div className="telegram-messages" ref={messageListRef}>
          {!selectedChat ? (
            <div className="telegram-hero">
              <Bot size={42} />
              <strong>选择一个对话开始浏览</strong>
              <span>这里是只读模式，不会发送任何消息。</span>
            </div>
          ) : loadingMessages ? (
            <div className="telegram-hero">
              <LoaderCircle size={34} />
              <strong>正在读取消息</strong>
              <span>tdl 正在导出最近 {messageCount} 条消息。</span>
            </div>
          ) : messages.length ? (
            messages.map((item) => (
              <MessageBubble
                key={item.id}
                message={item}
                selected={selectedMessageIds.has(item.id)}
                previewState={mediaPreviews[item.id] ?? { status: "idle" }}
                nodeRef={(node) => registerMessageNode(item.id, node)}
                onToggle={() => onToggleMessage(item.id)}
                onPreview={() => onPreviewMessage(item)}
              />
            ))
          ) : (
            <div className="telegram-hero">
              <MessageSquareText size={38} />
              <strong>没有可显示的消息</strong>
              <span>可以刷新消息或增加最近消息数量。</span>
            </div>
          )}
        </div>
      </section>
    </div>
  );
}

function MessageBubble({
  message,
  selected,
  previewState,
  nodeRef,
  onToggle,
  onPreview,
}: {
  message: MessageInfo;
  selected: boolean;
  previewState: MediaPreviewState;
  nodeRef: (node: HTMLElement | null) => void;
  onToggle: () => void;
  onPreview: () => void;
}) {
  const Icon = mediaIcon(message.mediaKind);
  const title =
    message.fileName ||
    message.mediaType ||
    (message.mediaKind !== "none" ? `${mediaKindLabel(message.mediaKind)}消息` : "消息");
  return (
    <article
      className={`message-bubble ${selected ? "selected" : ""}`}
      data-message-id={message.id}
      ref={nodeRef}
    >
      <label className="message-check">
        <input type="checkbox" checked={selected} onChange={onToggle} />
      </label>
      <div className="message-bubble-body">
        <div className="message-bubble-top">
          <strong>
            <Icon size={16} />
            {title}
          </strong>
          <span>#{message.id}</span>
        </div>
        <MediaPreview state={previewState} message={message} />
        <p>{message.text || (message.mediaKind !== "none" ? `${mediaKindLabel(message.mediaKind)}消息` : "无文字内容")}</p>
        <div className="message-bubble-meta">
          <span>{formatDate(message.date)}</span>
          {message.fileSize ? <span>{formatFileSize(message.fileSize)}</span> : null}
          {message.mimeType ? <span>{message.mimeType}</span> : null}
          {message.mediaType ? <span>{message.mediaType}</span> : null}
          {message.width && message.height ? <span>{message.width}x{message.height}</span> : null}
          {message.duration ? <span>{Math.round(message.duration)}s</span> : null}
        </div>
        {message.previewable ? (
          <button
            className="preview-button"
            type="button"
            onClick={onPreview}
            disabled={previewState.status === "loading"}
          >
            {previewState.status === "loading" ? <LoaderCircle size={15} /> : <Eye size={15} />}
            {previewButtonLabel(message, previewState)}
          </button>
        ) : null}
        {previewState.status === "error" ? <div className="preview-error">{previewState.error}</div> : null}
      </div>
    </article>
  );
}

function MediaPreview({ state, message }: { state: MediaPreviewState; message: MessageInfo }) {
  if (state.status !== "ready" || state.preview.files.length === 0) {
    if (message.mediaKind === "none") return null;
    return <MediaPlaceholder message={message} state={state} />;
  }

  return (
    <div className="media-preview-grid">
      {state.preview.files.map((file) => (
        <MediaPreviewFile key={file.path} file={file} messageId={message.id} />
      ))}
    </div>
  );
}

function MediaPlaceholder({ message, state }: { message: MessageInfo; state: MediaPreviewState }) {
  const Icon = mediaIcon(message.mediaKind);
  const name = message.fileName || `${mediaKindLabel(message.mediaKind)}消息`;
  const text =
    state.status === "loading"
      ? "正在加载缩略图"
      : state.status === "skipped"
        ? message.mediaKind === "video"
          ? "视频较大，点击加载预览"
          : "文件较大，点击加载缩略图"
        : canPreviewMedia(message.mediaKind)
          ? "进入可见区域后自动加载"
          : "媒体文件";

  return (
    <div className="media-preview-grid">
      <div className={`media-thumb-card media-thumb-placeholder media-thumb-${message.mediaKind}`}>
        <div className="media-thumb-visual">
          <Icon size={28} />
          {state.status === "loading" ? <LoaderCircle className="media-thumb-loader" size={18} /> : null}
        </div>
        <div className="media-thumb-info">
          <strong>#{message.id}</strong>
          <span>{name}</span>
          <em>{text}</em>
        </div>
      </div>
    </div>
  );
}

function MediaPreviewFile({ file, messageId }: { file: ChatMediaPreviewFile; messageId: number }) {
  const source = convertFileSrc(file.path);
  const Icon = mediaIcon(file.mediaKind);
  const title = `#${messageId} ${file.fileName}`;

  return (
    <div className={`media-thumb-card media-thumb-${file.mediaKind}`} title={title}>
      <div className="media-thumb-visual">
        {file.mediaKind === "photo" ? (
          <img src={source} alt={title} loading="lazy" draggable={false} />
        ) : file.mediaKind === "video" ? (
          <>
            <video src={source} muted playsInline preload="metadata" />
            <span className="media-thumb-overlay">
              <Video size={16} />
            </span>
          </>
        ) : (
          <Icon size={24} />
        )}
      </div>
      <div className="media-thumb-info">
        <strong>#{messageId}</strong>
        <span>{file.fileName}</span>
        {file.size ? <em>{formatFileSize(file.size)}</em> : null}
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
