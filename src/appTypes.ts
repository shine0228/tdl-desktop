import type { ChatMediaPreview, LinkPreview, MessageInfo, SourceMode } from "./types";

export type PreviewState =
  | { status: "idle" }
  | { status: "loading"; link: string }
  | { status: "ready"; link: string; preview: LinkPreview }
  | { status: "error"; link: string; error: string };

export type MediaPreviewState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; preview: ChatMediaPreview }
  | { status: "skipped"; reason: "tooLarge" | "videoTooLarge" | "unsupported" }
  | { status: "error"; error: string };

export type QueuedPreview = {
  chatId: string;
  generation: number;
  item: MessageInfo;
};

export type AppMode = SourceMode | "history" | "settings" | "docs";
export type AppLanguage = "zh" | "en";
