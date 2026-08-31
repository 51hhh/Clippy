import { Copy, Ellipsis, Image, Star, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { getClipThumbnail, type ClipItem } from "../../js/api.ts";
import { formatRelativeTime, formatSize } from "../../js/clipboard/formatters.js";
import { t } from "../shared/i18n";
import type { ClipboardSnapshot } from "./clipboardStore";

type Action = "copy" | "favorite" | "delete";

export function ClipboardRow({
  clip,
  index,
  snapshot,
  onFocus,
  onToggle,
  onAction,
}: {
  clip: ClipItem;
  index: number;
  snapshot: ClipboardSnapshot;
  onFocus: () => void;
  onToggle: () => void;
  onAction: (action: Action, actionIndex: number) => void;
}) {
  const [imageBase64, setImageBase64] = useState<string | null>(null);
  const focused = snapshot.navigation.focusedRow === index;
  const expanded = snapshot.navigation.expandedRow === clip.id;
  const favoriteMode = snapshot.mode === "favorites";
  // 行里不显示内容类型（既不是 badge 也不进 meta）：后端 content_type 只有
  // text/html/image 三档，而右侧预览按内容嗅探，同一条会一边写 HTML 一边写 YAML。
  // 类型统一由 preview/classify.js 判定，只显示在预览面板的 badge 上。
  // 富文本没有纯文本副本时仍给一句占位——那是"内容为空"的提示，不是类型标签。
  const previewText = clip.content_type === "html"
    ? (clip.text_content || t("preview.richText"))
    : (clip.text_content || "");

  useEffect(() => {
    let cancelled = false;
    if (clip.content_type !== "image") return;
    // 缩略图而不是原图：这一格是 48×48，取原图等于为了画 48 px 把几 MB 的 PNG
    // 送进 webview 再全尺寸解码一次，而列表里可能同时有十几个图片条目。
    getClipThumbnail(clip.id)
      .then((value) => !cancelled && setImageBase64(value))
      .catch(() => undefined);
    return () => { cancelled = true; };
  }, [clip.content_type, clip.id]);

  const actions: Array<{ key: Action; label: string; icon: React.ReactNode }> = [
    { key: "copy", label: t("action.copy"), icon: <Copy size={16} /> },
    {
      key: "favorite",
      label: t(clip.is_favorite ? "action.unfavorite" : "action.favorite"),
      icon: <Star size={16} fill={clip.is_favorite ? "currentColor" : "none"} />,
    },
    { key: "delete", label: t("action.delete"), icon: <Trash2 size={16} /> },
  ];

  return (
    <div
      className={[
        "clip-row",
        focused ? "focused" : "",
        expanded ? "expanded" : "",
        favoriteMode ? "favorites-mode" : "",
        clip.is_favorite ? "favorite" : "",
        clip.is_sensitive ? "sensitive" : "",
      ].filter(Boolean).join(" ")}
      role="option"
      aria-selected={focused}
      data-id={clip.id}
      data-idx={index}
      onPointerMove={onFocus}
      onClick={() => onAction("copy", -1)}
    >
      <div className="clip-row-main">
        <div className={`clip-row-preview clip-row-preview--${clip.content_type}`}>
          {clip.content_type === "image" ? (
            <span className="clip-row-thumb">
              {imageBase64
                ? <img className="clip-row-thumb-img" src={`data:image/png;base64,${imageBase64}`} alt={t("preview.image")} draggable={false} />
                : <Image size={20} />}
            </span>
          ) : (
            <span>{previewText.slice(0, 200)}</span>
          )}
        </div>
        <div className="clip-row-meta">
          {formatSize(clip.byte_size)} · {formatRelativeTime(
            clip.created_at,
            { translate: (key: string, params?: object) => t(key, params as Record<string, string | number>) },
          )}
        </div>
      </div>
      <div className="clip-row-actions">
        {actions.map((action, actionIndex) => (
          <button
            key={action.key}
            type="button"
            className={[
              "clip-row-action",
              action.key === "favorite" && clip.is_favorite ? "is-favorite" : "",
              focused && snapshot.navigation.focusedCol === actionIndex ? "focused" : "",
            ].filter(Boolean).join(" ")}
            aria-label={action.label}
            title={action.label}
            onClick={(event) => {
              event.stopPropagation();
              onAction(action.key, actionIndex);
            }}
          >
            <span className="clip-row-action-icon">{action.icon}</span>
          </button>
        ))}
      </div>
      <button
        type="button"
        className="clip-row-trigger"
        aria-label={t("action.more")}
        title={t("action.more")}
        onClick={(event) => {
          event.stopPropagation();
          onToggle();
        }}
      >
        <Ellipsis size={16} />
      </button>
    </div>
  );
}
