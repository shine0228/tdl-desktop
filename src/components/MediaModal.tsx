import { XCircle } from "lucide-react";
import { useEffect, useRef } from "react";

import type { MediaKind } from "../types";

export type FullMedia = { src: string; kind: MediaKind; title: string };

export function MediaModal({ media, onClose }: { media: FullMedia; onClose: () => void }) {
  const contentRef = useRef<HTMLDivElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeButtonRef.current?.focus();

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
        return;
      }
      if (event.key !== "Tab") return;

      const focusable = getFocusableElements(contentRef.current);
      if (focusable.length === 0) {
        event.preventDefault();
        return;
      }

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      previousFocus?.focus();
    };
  }, [onClose]);

  return (
    <div className="image-modal-overlay" role="dialog" aria-modal="true" aria-label={media.title} onClick={onClose}>
      <div ref={contentRef} className="image-modal-content" onClick={(event) => event.stopPropagation()}>
        {media.kind === "video" ? (
          <video src={media.src} controls autoPlay playsInline />
        ) : (
          <img src={media.src} alt={media.title} />
        )}
        <button ref={closeButtonRef} className="image-modal-close" onClick={onClose} aria-label="Close media preview">
          <XCircle size={24} />
        </button>
      </div>
    </div>
  );
}

function getFocusableElements(root: HTMLElement | null) {
  if (!root) return [];
  return Array.from(
    root.querySelectorAll<HTMLElement>(
      'button, [href], input, select, textarea, video[controls], [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((element) => !element.hasAttribute("disabled") && !element.getAttribute("aria-hidden"));
}
