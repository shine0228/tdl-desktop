import { invoke } from "@tauri-apps/api/core";
import {
  Bot,
  ChevronDown,
  Copy,
  File,
  FileJson,
  FolderOpen,
  Link as LinkIcon,
  ListChecks,
  Play,
  RefreshCcw,
  Settings2,
  SlidersHorizontal,
  Square,
  Terminal,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { AppMode, MediaPreviewState, PreviewState, QueuedPreview } from "./appTypes";
import { canPreviewMedia, modeLabel, shouldAutoDownloadPreview, skippedPreviewState } from "./appUtils";
import { DocumentationWorkspace } from "./components/DocumentationWorkspace";
import { FileProgressList } from "./components/FileProgressList";
import { HistoryItem } from "./components/HistoryItem";
import { LoginPanel } from "./components/LoginPanel";
import { MediaModal, type FullMedia } from "./components/MediaModal";
import { Metric } from "./components/Metric";
import { SettingsWorkspace } from "./components/SettingsWorkspace";
import { TelegramWorkspace, LinkPreviewPanel } from "./components/TelegramWorkspace";
import { Toggle } from "./components/Toggle";
import { useTauriEvent } from "./hooks/useTauriEvent";
import { TRANSLATIONS, type TranslationKey } from "./i18n";
import type {
  AppConfig,
  AppState,
  ChatDownloadRequest,
  ChatInfo,
  ChatMediaPreview,
  DownloadEvent,
  DownloadFileProgress,
  DownloadRecord,
  DownloadRequest,
  DownloadStarted,
  DownloadStatus,
  LoginEvent,
  LoginMethod,
  LoginStarted,
  LoginStatus,
  LinkPreview,
  MessageInfo,
  SourceMode,
  TdlInfo,
  TdlUpdateEvent,
  LogPackageInfo,
  DesktopUpdateStatus,
} from "./types";

const DEFAULT_CONFIG: AppConfig = {
  lastDirectory: "",
  limit: 4,
  threads: 4,
  pool: 8,
  tdlOverridePath: null,
  language: "zh",
  logDirectory: "",
  desktopUpdateUrl: "",
  tdlNamespace: "default",
  tdlStorage: "",
};

const AUTO_CHAT_REFRESH_MS = 60_000;
const AUTO_PREVIEW_CONCURRENCY = 1;

const DOWNLOAD_PRESETS = [
  { key: "stable", label: "presetStable", limit: 2, threads: 2, pool: 4 },
  { key: "balanced", label: "presetBalanced", limit: 4, threads: 4, pool: 8 },
  { key: "fast", label: "presetFast", limit: 8, threads: 8, pool: 16 },
  { key: "max", label: "presetMax", limit: 16, threads: 16, pool: 32 },
] as const satisfies ReadonlyArray<{
  key: string;
  label: TranslationKey;
  limit: number;
  threads: number;
  pool: number;
}>;

function inTauri() {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

function splitLines(value: string) {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
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

function buildLocalPreview(request: DownloadRequest, t: (key: TranslationKey) => string) {
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
    quoteArg(request.directory || `<${t("downloadDirectory")}>`),
    "-l",
    String(request.limit),
    "-t",
    String(request.threads),
  ];

  if (request.pool > 0) {
    args.push("--pool", String(request.pool));
  }

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
  const rawArgs = "download ";
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
  const [linkPreview, setLinkPreview] = useState<PreviewState>({ status: "idle" });
  const [progress, setProgress] = useState<number | null>(null);
  const [fileProgresses, setFileProgresses] = useState<DownloadFileProgress[]>([]);
  const [loginStatus, setLoginStatus] = useState<LoginStatus>(
    loggedOutStatus(TRANSLATIONS.zh.loginUnchecked),
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
  const [message, setMessage] = useState<string>(TRANSLATIONS.zh.loading);
  const [busy, setBusy] = useState(false);
  const [tdlUpdateChecking, setTdlUpdateChecking] = useState(false);
  const [tdlUpdating, setTdlUpdating] = useState(false);
  const [desktopUpdateChecking, setDesktopUpdateChecking] = useState(false);
  const [logPackage, setLogPackage] = useState<LogPackageInfo | null>(null);
  const [desktopUpdateStatus, setDesktopUpdateStatus] = useState<DesktopUpdateStatus | null>(null);
  const [desktopVersion, setDesktopVersion] = useState("");
  const [fullMedia, setFullMedia] = useState<FullMedia | null>(null);
  const [lastCompletedTask, setLastCompletedTask] = useState<{
    recordIds: string[];
    status: DownloadStatus;
    directory: string;
  } | null>(null);

  const language = config.language === "en" ? "en" : "zh";
  const t = useCallback((key: TranslationKey) => TRANSLATIONS[language][key], [language]);

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
    async (next: AppConfig, options?: { immediate?: boolean }): Promise<void> => {
      setConfig(next);
      pendingConfigRef.current = next;
      if (!inTauri()) return Promise.resolve();
      if (options?.immediate) {
        await flushConfig();
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
  }, []);

  useTauriEvent<DownloadEvent>("download-event", handleDownloadEvent);
  useTauriEvent<LoginEvent>("login-event", handleLoginEvent);
  useTauriEvent<TdlUpdateEvent>("tdl-update-event", handleTdlUpdateEvent);

  useEffect(() => {
    if (!config.lastDirectory) return;
    setDirectory((current) => current || config.lastDirectory);
  }, [config.lastDirectory]);

  useEffect(() => {
    if (!inTauri() || !loginStatus.loggedIn) return;
    void loadChats({ silent: true });
    const timer = window.setInterval(() => {
      void loadChats({ silent: true });
    }, AUTO_CHAT_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [loginStatus.loggedIn]);

  useEffect(() => {
    if (!inTauri() || mode !== "chat" || !loginStatus.loggedIn || chats.length > 0) return;
    void loadChats({ silent: true });
  }, [chats.length, loginStatus.loggedIn, mode]);

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
      setLinkPreview({ status: "error", link: candidate, error: t("previewUnavailableDetail") });
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
  }, [linksText, mode, t, tdl]);

  const downloadMode: SourceMode = mode === "history" || mode === "settings" || mode === "docs" ? "links" : mode;
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

  const commandPreview = useMemo(() => buildLocalPreview(request, t), [request, t]);

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
      setMessage(t("frontendPreviewMode"));
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
      setDesktopVersion(state.desktopVersion);
      setMessage(state.tdl.available ? t("ready") : t("tdlUnavailable"));
      if (state.tdl.available) {
        void refreshLoginStatus().then((status) => {
          if (status?.loggedIn) {
            void loadChats({ silent: true });
          }
        });
      } else {
        setLoginStatus(loggedOutStatus(t("tdlCannotCheckLogin")));
      }
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function refreshLoginStatus(options?: { verifyOnline?: boolean }) {
    if (!inTauri()) return null;
    setLoginChecking(true);
    try {
      const status = await invoke<LoginStatus>("check_login_status", options?.verifyOnline ? { request: { verifyOnline: true } } : {});
      setLoginStatus(status);
      return status;
    } catch (error) {
      const status = loggedOutStatus(t("cannotCheckLogin"), String(error));
      setLoginStatus(status);
      return status;
    } finally {
      setLoginChecking(false);
    }
  }

  function handleDownloadEvent(event: DownloadEvent) {
    if (event.kind === "output" && event.line) {
      setLogs((current) => [...current.slice(-120), event.line as string]);
      if (event.fileProgress) {
        mergeFileProgress(event.fileProgress);
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
      setHistory((current) => {
        const updated = current.map((record) =>
          event.recordIds.includes(record.id)
            ? {
                ...record,
                status: event.status ?? record.status,
                completedAt: event.completedAt,
                error: event.error,
              }
            : record,
        );

        // 保存最近完成任务信息用于结果操作区
        const completedRecord = updated.find((r) => event.recordIds.includes(r.id));
        if (completedRecord && event.status) {
          setLastCompletedTask({
            recordIds: event.recordIds,
            status: event.status,
            directory: completedRecord.directory,
          });
        }

        return updated;
      });
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
        setLoginStatus(loggedOutStatus(event.message ?? t("loginComplete")));
        void refreshLoginStatus().then((status) => {
          if (status?.loggedIn) {
            void loadChats({ silent: true });
          }
        });
      } else {
        setLoginStatus(loggedOutStatus(event.message ?? t("loginFailed"), event.error));
      }
      setMessage(event.message ?? "");
    }
  }

  function handleTdlUpdateEvent(event: TdlUpdateEvent) {
    setTdlUpdateChecking(false);
    setTdlUpdating(false);
    if (event.tdl) {
      setTdl(event.tdl);
    }
    setMessage(event.message);
  }

  async function saveConfig(next: AppConfig) {
    await persistConfig(next, { immediate: true });
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
      setMessage(t("loadingChatList"));
    }
    try {
      const items = await invoke<ChatInfo[]>("list_chats");
      setChats(items);
      if (!options?.silent) {
        setMessage(`${items.length} ${t("chatsLoadedSuffix")}`);
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
    setMessage(`${t("readingMessages")} ${chat.name} · ${messageCount} ${t("message")}`);
    try {
      const items = await invoke<MessageInfo[]>("export_chat_messages", {
        chatId: String(chat.id),
        count: messageCount,
      });
      setMessages(items);
      setMessage(`${items.length} ${t("messagesReadSuffix")}`);
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
    if (!directory.trim() && mode !== "raw") {
      setMessage(t("chooseDownloadDirectory"));
      return;
    }
    if (mode === "chat" && (!selectedChat || selectedMessageIds.size === 0)) {
      setMessage(t("chooseChatAndMessages"));
      return;
    }

    setBusy(true);
    setLogs([]);
    setProgress(0);
    setFileProgresses([]);
    setMessage(t("startDownload"));

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
      setMessage(t("downloading"));
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
    setMessage(t("cancelling"));
  }

  async function clearCache() {
    if (!window.confirm(t("clearCacheConfirm"))) return;
    previewGenerationRef.current += 1;
    previewQueueRef.current = [];
    queuedPreviewIdsRef.current.clear();
    checkedPreviewCacheIdsRef.current.clear();
    setChats([]);
    setSelectedChat(null);
    setMessages([]);
    setSelectedMessageIds(new Set());
    setMediaPreviews({});
    try {
      if (inTauri()) {
        await invoke("clear_chat_cache");
      }
      setMessage(t("cacheCleared"));
    } catch (error) {
      setMessage(`${t("cacheClearFailed")}: ${String(error)}`);
    }
  }

  async function checkTdlUpdate() {
    if (!inTauri() || tdlUpdateChecking) return;
    setTdlUpdateChecking(true);
    try {
      const info = await invoke<TdlInfo>("refresh_tdl_info");
      setTdl(info);
      setMessage(t("tdlRefreshed"));
    } catch (error) {
      setMessage(String(error));
    } finally {
      setTdlUpdateChecking(false);
    }
  }

  async function updateTdl() {
    if (!inTauri() || tdlUpdating) return;
    setTdlUpdating(true);
    setMessage(t("updatingTdl"));
    try {
      await invoke("update_tdl");
      // 实际更新结果通过 tdl-update-event 返回
    } catch (error) {
      setMessage(String(error));
      setTdlUpdating(false);
    }
  }

  async function checkDesktopUpdate() {
    if (!inTauri() || desktopUpdateChecking) return;
    setDesktopUpdateChecking(true);
    try {
      const status = await invoke<DesktopUpdateStatus>("check_desktop_update");
      setDesktopUpdateStatus(status);
      setMessage(status.message);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setDesktopUpdateChecking(false);
    }
  }

  async function pickLogDirectory() {
    if (!inTauri()) return;
    const picked = await invoke<string | null>("pick_log_directory");
    if (picked) {
      await saveConfig({ ...config, logDirectory: picked });
    }
  }

  async function collectLogs() {
    if (!inTauri()) return;
    try {
      const info = await invoke<LogPackageInfo>("collect_logs");
      setLogPackage(info);
      setMessage(info.message);
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function startLogin(method: LoginMethod) {
    if (!inTauri() || loginRunning) return;
    setLoginRunning(true);
    setLoginQr("");
    setLoginLogs([]);
    setLoginStatus(
      loggedOutStatus(method === "desktop" ? t("connectingTelegramDesktop") : t("generatingQrLogin")),
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
          ? t("connectingViaDesktop")
          : t("scanQrPrompt"),
      ]);
    } catch (error) {
      setLoginRunning(false);
      setLoginStatus(loggedOutStatus(t("startLoginFailed"), String(error)));
      setMessage(String(error));
    }
  }

  async function cancelLogin() {
    if (!inTauri() || !loginRunning) return;
    await invoke("cancel_login");
    setLoginStatus(loggedOutStatus(t("cancellingLogin")));
  }

  async function logout() {
    if (!inTauri() || loginRunning) return;
    if (!window.confirm(t("logoutConfirm"))) return;
    setBusy(true);
    setMessage(t("loggingOut"));
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

  async function openRecordDirectory(record: DownloadRecord) {
    if (!inTauri() || !record.directory) return;
    try {
      await invoke("open_directory", { path: record.directory });
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function copyRecordError(record: DownloadRecord) {
    if (!record.error) return;
    try {
      await navigator.clipboard.writeText(record.error);
      setMessage(t("copyError") + ": " + record.error.slice(0, 50));
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function copyRecordSource(record: DownloadRecord) {
    try {
      await navigator.clipboard.writeText(record.source);
      setMessage(t("copySource") + ": " + record.source.slice(0, 50));
    } catch (error) {
      setMessage(String(error));
    }
  }

  function useRecordAsInput(record: DownloadRecord) {
    if (record.mode === "links") {
      setMode("links");
      setLinksText(record.source);
      setDirectory(record.directory);
      setMessage(t("useAsInput") + " - " + t("linkDownload"));
    } else if (record.mode === "json") {
      setMode("json");
      setFilesText(record.source);
      setDirectory(record.directory);
      setMessage(t("useAsInput") + " - " + t("jsonImport"));
    }
  }

  async function retryDownload(record: DownloadRecord) {
    if (!inTauri() || !record.request) {
      setMessage("无法重试：缺少原始请求参数");
      return;
    }

    try {
      setBusy(true);
      const request = record.request as DownloadRequest | ChatDownloadRequest;

      if (record.mode === "chat") {
        const chatRequest = request as ChatDownloadRequest;
        const result = await invoke<DownloadStarted>("download_from_chat", { request: chatRequest });
        setRunningTaskId(result.taskId);
        setHistory((current) => [...result.records, ...current]);
        setMessage(t("startDownloadTask"));
      } else {
        const downloadRequest = request as DownloadRequest;
        const result = await invoke<DownloadStarted>("start_download", { request: downloadRequest });
        setRunningTaskId(result.taskId);
        setHistory((current) => [...result.records, ...current]);
        setMessage(t("startDownloadTask"));
      }
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function continueDownload(record: DownloadRecord) {
    if (!inTauri() || !record.request) {
      setMessage("无法继续：缺少原始请求参数");
      return;
    }

    try {
      setBusy(true);
      const request = record.request as DownloadRequest | ChatDownloadRequest;
      const modifiedRequest = { ...request, continueLast: true, restart: false };

      if (record.mode === "chat") {
        const result = await invoke<DownloadStarted>("download_from_chat", { request: modifiedRequest });
        setRunningTaskId(result.taskId);
        setHistory((current) => [...result.records, ...current]);
        setMessage(t("continueDownload"));
      } else {
        const result = await invoke<DownloadStarted>("start_download", { request: modifiedRequest });
        setRunningTaskId(result.taskId);
        setHistory((current) => [...result.records, ...current]);
        setMessage(t("continueDownload"));
      }
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  const running = Boolean(runningTaskId);
  const completed = history.filter((item) => item.status === "completed").length;
  const failed = history.filter((item) => item.status === "failed").length;
  const latestLog = logs.at(-1) ?? (running ? t("waitingTdlOutput") : t("waitingTaskOutput"));
  const progressValue = progress ?? 0;
  const completedFiles = fileProgresses.filter((item) => item.done || item.progress >= 99.9).length;
  const progressSummary = fileProgresses.length
    ? `${completedFiles}/${fileProgresses.length} ${t("filesUnit")}`
    : progress !== null
      ? `${Math.round(progress)}%`
      : running
        ? t("running")
        : "-";
  return (
    <main className="app-shell">
      <aside className="app-sidebar">
        <header className="sidebar-header">
          <h1>TDL Desktop</h1>
        </header>

        <nav className="nav-menu">
          <button className={`nav-item ${mode === "links" ? "active" : ""}`} onClick={() => setMode("links")}>
            <LinkIcon size={18} />
            {t("linkDownload")}
          </button>
          <button className={`nav-item ${mode === "json" ? "active" : ""}`} onClick={() => setMode("json")}>
            <FileJson size={18} />
            {t("jsonImport")}
          </button>
          <button className={`nav-item ${mode === "chat" ? "active" : ""}`} onClick={() => setMode("chat")}>
            <Bot size={18} />
            {t("chatBrowse")}
          </button>
          <button className={`nav-item ${mode === "history" ? "active" : ""}`} onClick={() => setMode("history")}>
            <ListChecks size={18} />
            {t("taskHistory")}
          </button>
          <button className={`nav-item ${mode === "docs" ? "active" : ""}`} onClick={() => setMode("docs")}>
            <File size={18} />
            {t("operationDoc")}
          </button>
          <button className={`nav-item ${mode === "settings" ? "active" : ""}`} onClick={() => setMode("settings")}>
            <SlidersHorizontal size={18} />
            {t("settings")}
          </button>
        </nav>
      </aside>

      <section className="main-workspace">
        <header className="topbar">
          <div className="status-line">
            <span className={`status-dot ${tdl?.available ? "ready" : "error"}`} />
            <span>{tdl?.available ? t("tdlAvailable") : t("tdlUnavailable")}</span>
          </div>
          <div className="topbar-actions">
            <button className="ghost-button" onClick={loadState} disabled={busy}>
              <RefreshCcw size={16} />
              {t("refresh")}
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
              onMediaClick={setFullMedia}
              language={language}
              t={t}
            />
          ) : mode === "history" ? (
            <section className="history-section" style={{ border: "none", boxShadow: "none", borderRadius: 0, padding: "40px" }}>
              <div className="section-header">
                <h2>{t("downloadHistory")}</h2>
                <button className="ghost-button" onClick={clearHistory}>
                  <Trash2 size={16} />
                  {t("clearHistory")}
                </button>
              </div>
              <div className="history-list">
                {history.length ? (
                  history.map((record) => (
                    <HistoryItem
                      key={record.id}
                      record={record}
                      language={language}
                      t={t}
                      onOpenDirectory={openRecordDirectory}
                      onCopyError={copyRecordError}
                      onCopySource={copyRecordSource}
                      onUseAsInput={useRecordAsInput}
                      onRetry={retryDownload}
                      onContinue={continueDownload}
                    />
                  ))
                ) : (
                  <div className="empty-state">
                    <ListChecks size={24} />
                    <span>{t("noDownloadRecords")}</span>
                  </div>
                )}
              </div>
            </section>
          ) : mode === "docs" ? (
            <DocumentationWorkspace title={t("operationDoc")} />
          ) : mode === "settings" ? (
            <SettingsWorkspace
              config={config}
              tdl={tdl}
              t={t}
              logPackage={logPackage}
              desktopUpdateStatus={desktopUpdateStatus}
              desktopVersion={desktopVersion}
              desktopUpdateChecking={desktopUpdateChecking}
              tdlUpdateChecking={tdlUpdateChecking}
              tdlUpdating={tdlUpdating}
              onSaveConfig={saveConfig}
              onPickLogDirectory={() => void pickLogDirectory()}
              onCollectLogs={() => void collectLogs()}
              onCheckDesktopUpdate={() => void checkDesktopUpdate()}
              onCheckTdlUpdate={() => void checkTdlUpdate()}
              onUpdateTdl={() => void updateTdl()}
              onClearCache={() => void clearCache()}
            />
          ) : (
            <div className="content-grid">
              <section className="task-panel">
                <div className="section-header">
                  <h2>{t(modeLabel[mode])} {t("downloadTask")}</h2>
                  <span>{message}</span>
                </div>

                {mode === "links" ? (
                  <>
                    <label className="field">
                      <span>{t("messageLinks")}</span>
                      <textarea
                        value={linksText}
                        onChange={(event) => setLinksText(event.target.value)}
                        spellCheck={false}
                        placeholder="https://t.me/channel/123"
                      />
                    </label>
                    <LinkPreviewPanel state={linkPreview} t={t} />
                  </>
                ) : null}

                {mode === "json" ? (
                  <label className="field">
                    <span>{t("exportedFilePath")}</span>
                    <textarea
                      value={filesText}
                      onChange={(event) => setFilesText(event.target.value)}
                      spellCheck={false}
                      placeholder="D:\\Downloads\\result.json"
                    />
                  </label>
                ) : null}

                <div className="action-row">
                  <button className="primary-button" onClick={startDownload} disabled={busy || running}>
                    <Play size={17} />
                    {t("startDownloadTask")}
                  </button>
                  <button className="danger-button" onClick={cancelDownload} disabled={!running}>
                    <Square size={16} />
                    {t("stopCurrentTask")}
                  </button>
                </div>

                <div className="directory-row">
                  <label className="field compact" style={{ flex: 1 }}>
                    <span>{t("localDownloadDir")}</span>
                    <input value={directory} onChange={(event) => setDirectory(event.target.value)} />
                  </label>
                  <button className="icon-button" onClick={pickDirectory} title={t("chooseDirectory")} style={{ marginTop: "24px" }}>
                    <FolderOpen size={18} />
                  </button>
                </div>

                <div className="parameter-presets">
                  <span>{t("downloadPresets")}</span>
                  <div className="segmented">
                    {DOWNLOAD_PRESETS.map((preset) => {
                      const isActive =
                        config.limit === preset.limit &&
                        config.threads === preset.threads &&
                        config.pool === preset.pool;
                      return (
                        <button
                          key={preset.key}
                          type="button"
                          className={isActive ? "active" : ""}
                          onClick={() =>
                            void persistConfig(
                              { ...config, limit: preset.limit, threads: preset.threads, pool: preset.pool },
                              { immediate: true }
                            )
                          }
                        >
                          {t(preset.label)}
                        </button>
                      );
                    })}
                  </div>
                </div>

                <div className="number-grid">
                  <label className="field compact">
                    <span>{t("concurrentTasks")}</span>
                    <input
                      type="number"
                      min={1}
                      max={32}
                      value={config.limit}
                      onChange={(event) =>
                        void persistConfig({ ...config, limit: Number(event.target.value) })
                      }
                      onBlur={() => void flushConfig()}
                    />
                  </label>
                  <label className="field compact">
                    <span>{t("fileThreads")}</span>
                    <input
                      type="number"
                      min={1}
                      max={32}
                      value={config.threads}
                      onChange={(event) =>
                        void persistConfig({ ...config, threads: Number(event.target.value) })
                      }
                      onBlur={() => void flushConfig()}
                    />
                  </label>
                  <label className="field compact">
                    <span>{t("dcPool")}</span>
                    <input
                      type="number"
                      min={0}
                      max={64}
                      value={config.pool}
                      onChange={(event) =>
                        void persistConfig({ ...config, pool: Number(event.target.value) })
                      }
                      onBlur={() => void flushConfig()}
                    />
                  </label>
                </div>

                <button className="advanced-toggle" onClick={() => setAdvancedOpen((value) => !value)}>
                  <Settings2 size={16} />
                  {t("advancedConfig")}
                  <ChevronDown size={16} className={advancedOpen ? "open" : ""} />
                </button>

                {advancedOpen ? (
                  <div className="advanced-grid">
                    <Toggle label={t("groupDownloadMode")} checked={group} onChange={setGroup} />
                    <Toggle label={t("skipSameName")} checked={skipSame} onChange={setSkipSame} />
                    <Toggle label={t("continueLast")} checked={continueLast} onChange={setContinueLast} />
                    <Toggle label={t("forceRestart")} checked={restart} onChange={setRestart} />
                    <Toggle label={t("descExport")} checked={desc} onChange={setDesc} />
                    <Toggle label={t("useTakeout")} checked={takeout} onChange={setTakeout} />
                    <Toggle label={t("rewriteExt")} checked={rewriteExt} onChange={setRewriteExt} />
                    <label className="field compact">
                      <span>{t("includeExt")}</span>
                      <input value={include} onChange={(event) => setInclude(event.target.value)} placeholder="mp4,jpg" />
                    </label>
                    <label className="field compact">
                      <span>{t("excludeExt")}</span>
                      <input value={exclude} onChange={(event) => setExclude(event.target.value)} placeholder="tmp,part" />
                    </label>
                    <label className="field compact wide">
                      <span>{t("filenameTemplate")}</span>
                      <input value={template} onChange={(event) => setTemplate(event.target.value)} />
                    </label>
                  </div>
                ) : null}

                <div className="command-preview">
                  <Terminal size={18} />
                  <code>{commandPreview}</code>
                </div>
              </section>
            </div>
          )}
        </div>
      </section>

      <aside className="app-status-panel">
        <div className="section-header">
          <h2>{t("statusAndProgress")}</h2>
        </div>

        <div className="metrics">
          <Metric label={t("totalTasks")} value={history.length} />
          <Metric label={t("completedTasks")} value={completed} tone="success" />
          <Metric label={t("failedTasks")} value={failed} tone="error" />
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
          onRefresh={() => void refreshLoginStatus({ verifyOnline: true })}
          onDesktopLogin={() => void startLogin("desktop")}
          onQrLogin={() => void startLogin("qr")}
          onCancel={() => void cancelLogin()}
          onLogout={() => void logout()}
          t={t}
        />

        {lastCompletedTask && (
          <div className="completion-result">
            <div className="section-header">
              <h2>
                {lastCompletedTask.status === "completed"
                  ? t("downloadCompleted")
                  : lastCompletedTask.status === "failed"
                    ? t("downloadFailed")
                    : t("downloadCancelled")}
              </h2>
            </div>
            {lastCompletedTask.status === "completed" && lastCompletedTask.directory && (
              <p className="result-directory">
                <strong>{t("savedTo")}:</strong> {lastCompletedTask.directory}
              </p>
            )}
            <div className="result-actions">
              {lastCompletedTask.directory && (
                <button
                  type="button"
                  className="ghost-button"
                  onClick={() => openRecordDirectory({ directory: lastCompletedTask.directory } as DownloadRecord)}
                >
                  <FolderOpen size={16} />
                  {t("openDirectory")}
                </button>
              )}
              {lastCompletedTask.directory && (
                <button
                  type="button"
                  className="ghost-button"
                  onClick={() => copyRecordSource({ source: lastCompletedTask.directory } as DownloadRecord)}
                >
                  <Copy size={16} />
                  {t("copyPath")}
                </button>
              )}
              <button
                type="button"
                className="ghost-button"
                onClick={() => {
                  setMode("history");
                  setLastCompletedTask(null);
                }}
              >
                <ListChecks size={16} />
                {t("viewHistory")}
              </button>
            </div>
          </div>
        )}

        <div className="activity-panel">
          <div className="section-header">
            <h2>{t("realtimeProgress")}</h2>
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
            <span style={{ fontSize: "12px", color: "var(--text-secondary)", lineHeight: "1.5" }}>{latestLog}</span>
          </div>
        </div>
      </aside>

      {fullMedia ? (
        <MediaModal media={fullMedia} onClose={() => setFullMedia(null)} />
      ) : null}
    </main>
  );
}

export default App;
