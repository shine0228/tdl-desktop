import type { DownloadFileProgress } from "../types";

export function FileProgressList({ items }: { items: DownloadFileProgress[] }) {
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
