import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

function inTauri() {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

export function useTauriEvent<T>(eventName: string, handler: (payload: T) => void) {
  const handlerRef = useRef(handler);

  useEffect(() => {
    handlerRef.current = handler;
  }, [handler]);

  useEffect(() => {
    if (!inTauri()) return;

    let cancelled = false;
    let unlisten: (() => void) | undefined;

    listen<T>(eventName, (event) => handlerRef.current(event.payload))
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((error) => {
        if (!cancelled) {
          console.error(`订阅 ${eventName} 失败`, error);
        }
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [eventName]);
}
