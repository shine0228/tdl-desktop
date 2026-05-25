import { convertFileSrc } from "@tauri-apps/api/core";
import {
  AlertCircle,
  Bot,
  CheckSquare2,
  Download,
  Eye,
  File,
  FolderOpen,
  Image as ImageIcon,
  LoaderCircle,
  MessageSquareText,
  Music,
  RefreshCcw,
  Search,
  Square,
  Video,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import type { AppLanguage, MediaPreviewState, PreviewState } from "../appTypes";
import {
  canPreviewMedia,
  formatDate,
  formatFileSize,
  loggedInLabel,
  mediaKindLabel,
  previewButtonLabel,
} from "../appUtils";
import type { FullMedia } from "./MediaModal";
import type { TranslationKey } from "../i18n";
import type { ChatInfo, ChatMediaPreviewFile, LoginStatus, MediaKind, MessageInfo } from "../types";

const AUTO_PREVIEW_LIMIT = 4;

function mediaIcon(kind: MediaKind) {
  if (kind === "photo") return ImageIcon;
  if (kind === "video") return Video;
  if (kind === "audio") return Music;
  if (kind === "document") return File;
  return MessageSquareText;
}

export function TelegramWorkspace({
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
  onMediaClick,
  language,
  t,
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
  onMediaClick?: (media: FullMedia) => void;
  language: AppLanguage;
  t: (key: TranslationKey) => string;
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
            <strong>{t("chats")}</strong>
            <span>{loginStatus.loggedIn ? loggedInLabel(loginStatus, t) : t("pleaseLoginTelegram")}</span>
          </div>
          <button className="icon-button compact-icon" onClick={onLoadChats} disabled={loadingChats} title={t("refreshChats")}>
            {loadingChats ? <LoaderCircle size={17} /> : <RefreshCcw size={17} />}
          </button>
        </div>
        <div className="telegram-search">
          <Search size={16} />
          <input
            value={chatSearch}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder={t("searchChats")}
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
            <div className="telegram-empty">
              {chatSearch.trim() ? t("noMatchedChats") : loadingChats ? t("loadingChatList") : t("refreshToLoadChats")}
            </div>
          )}
        </div>
      </aside>

      <section className="telegram-main">
        <header className="telegram-header">
          <div className="telegram-actions">
            <select
              value={messageCount}
              onChange={(event) => onMessageCountChange(Number(event.target.value))}
              disabled={loadingMessages}
              title={t("recentMessageCount")}
            >
              <option value={50}>{t("recent50")}</option>
              <option value={100}>{t("recent100")}</option>
              <option value={200}>{t("recent200")}</option>
            </select>
            <button className="ghost-button" onClick={onRefreshMessages} disabled={!selectedChat || loadingMessages}>
              {loadingMessages ? <LoaderCircle size={16} /> : <RefreshCcw size={16} />}
              {t("refreshMessages")}
            </button>
            <button className="ghost-button" onClick={onToggleAll} disabled={!messages.length || loadingMessages}>
              <CheckSquare2 size={16} />
              {allSelected ? t("deselectAll") : t("selectAll")}
            </button>
            <button className="primary-button" onClick={onStartDownload} disabled={!selectedChat || selectedCount === 0 || busy || running}>
              <Download size={17} />
              {selectedCount ? `${t("download")} ${selectedCount} ${t("message")}` : t("download")}
            </button>
            <button className="danger-button" onClick={onCancelDownload} disabled={!running}>
              <Square size={16} />
              {t("cancel")}
            </button>
          </div>
        </header>

        <div className="telegram-download-bar">
          <label className="field compact">
            <span>{t("downloadDirectory")}</span>
            <input value={directory} onChange={(event) => onDirectoryChange(event.target.value)} />
          </label>
          <button className="icon-button compact-icon" onClick={onPickDirectory} title={t("chooseDirectory")}>
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
              <strong>{t("selectChatToBrowse")}</strong>
              <span>{t("readOnlyNoSend")}</span>
            </div>
          ) : loadingMessages ? (
            <div className="telegram-hero">
              <LoaderCircle size={34} />
              <strong>{t("readingMessages")}</strong>
              <span>tdl {t("readingMessages")} {messageCount} {t("message")}.</span>
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
                onMediaClick={onMediaClick}
                language={language}
                t={t}
              />
            ))
          ) : (
            <div className="telegram-hero">
              <MessageSquareText size={38} />
              <strong>{t("noMessages")}</strong>
              <span>{t("refreshOrIncrease")}</span>
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
  onMediaClick,
  language,
  t,
}: {
  message: MessageInfo;
  selected: boolean;
  previewState: MediaPreviewState;
  nodeRef: (node: HTMLElement | null) => void;
  onToggle: () => void;
  onPreview: () => void;
  onMediaClick?: (media: FullMedia) => void;
  language: AppLanguage;
  t: (key: TranslationKey) => string;
}) {
  const Icon = mediaIcon(message.mediaKind);
  const title =
    message.fileName ||
    message.mediaType ||
    (message.mediaKind !== "none" ? `${mediaKindLabel(message.mediaKind, t)}${t("message")}` : t("message"));
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
        <MediaPreview state={previewState} message={message} onPreview={onPreview} onMediaClick={onMediaClick} t={t} />
        <p>{message.text || (message.mediaKind !== "none" ? `${mediaKindLabel(message.mediaKind, t)}${t("message")}` : t("noTextContent"))}</p>
        <div className="message-bubble-meta">
          <span>{formatDate(message.date, language)}</span>
          {message.fileSize ? <span>{formatFileSize(message.fileSize, t)}</span> : null}
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
            {previewButtonLabel(message, previewState, t)}
          </button>
        ) : null}
        {previewState.status === "error" ? <div className="preview-error">{previewState.error}</div> : null}
      </div>
    </article>
  );
}

function MediaPreview({
  state,
  message,
  onPreview,
  onMediaClick,
  t,
}: {
  state: MediaPreviewState;
  message: MessageInfo;
  onPreview: () => void;
  onMediaClick?: (media: FullMedia) => void;
  t: (key: TranslationKey) => string;
}) {
  if (state.status !== "ready" || state.preview.files.length === 0) {
    if (message.mediaKind === "none") return null;
    return <MediaPlaceholder message={message} state={state} onPreview={onPreview} t={t} />;
  }

  return (
    <div className="media-preview-grid">
      {state.preview.files.map((file) => (
        <MediaPreviewFile key={file.path} file={file} messageId={message.id} onClick={onMediaClick} t={t} />
      ))}
    </div>
  );
}

function MediaPlaceholder({
  message,
  state,
  onPreview,
  t,
}: {
  message: MessageInfo;
  state: MediaPreviewState;
  onPreview: () => void;
  t: (key: TranslationKey) => string;
}) {
  const Icon = mediaIcon(message.mediaKind);
  const name = message.fileName || `${mediaKindLabel(message.mediaKind, t)}${t("message")}`;
  const text =
    state.status === "loading"
      ? t("loadingThumbnail")
      : state.status === "skipped"
        ? message.mediaKind === "video"
          ? t("largeVideoPreview")
          : t("largeFileThumbnail")
        : canPreviewMedia(message.mediaKind)
          ? t("autoLoadVisible")
          : t("mediaFile");

  return (
    <div className="media-preview-grid" onClick={onPreview} role="button" tabIndex={0} onKeyDown={(event) => {
      if (event.key === "Enter" || event.key === " ") onPreview();
    }}>
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

function MediaPreviewFile({ file, messageId, onClick, t }: { file: ChatMediaPreviewFile; messageId: number; onClick?: (media: FullMedia) => void; t: (key: TranslationKey) => string }) {
  const source = convertFileSrc(file.path);
  const Icon = mediaIcon(file.mediaKind);
  const title = `#${messageId} ${file.fileName}`;

  return (
    <div className={`media-thumb-card media-thumb-${file.mediaKind}`} title={title} onClick={() => onClick?.({ src: source, kind: file.mediaKind, title })} style={{ cursor: onClick ? "pointer" : "default" }}>
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
        {file.size ? <em>{formatFileSize(file.size, t)}</em> : null}
      </div>
    </div>
  );
}

export function LinkPreviewPanel({ state, t }: { state: PreviewState; t: (key: TranslationKey) => string }) {
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
            <strong>{t("readingMessageText")}</strong>
            <span>{state.link}</span>
          </div>
          <p>{t("previewRequiresLogin")}</p>
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
            <strong>{t("previewUnavailable")}</strong>
            <span>{state.link}</span>
          </div>
          <p>{state.error}</p>
        </div>
      </div>
    );
  }

  const { preview } = state;
  const text = preview.text?.trim() || t("noRecognizedText");

  return (
    <div className="link-preview-panel">
      <div className="link-preview-icon">
        <MessageSquareText size={17} />
      </div>
      <div className="link-preview-main">
        <div className="link-preview-title">
          <strong>@{preview.chat}/{preview.messageId}</strong>
          <span>{preview.mediaCount > 0 ? `${t("mediaFields")} ${preview.mediaCount}` : t("textPreview")}</span>
        </div>
        <p>{text}</p>
      </div>
    </div>
  );
}
