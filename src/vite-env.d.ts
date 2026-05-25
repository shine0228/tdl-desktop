/// <reference types="vite/client" />

interface Window {
  __TAURI_INTERNALS__?: unknown;
}

declare module "*.md?raw" {
  const content: string;
  export default content;
}
