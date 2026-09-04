import { Copy, Ellipsis, Image, Star, Trash2 } from "lucide-react";
import { memo, useEffect, useRef, useState } from "react";
import { getClipThumbnail, type ClipItem } from "../../js/api.ts";
import { formatRelativeTime, formatSize } from "../../js/clipboard/formatters.js";
import { t } from "../shared/i18n";

type Action = "copy" | "favorite" | "delete";

/**
 * 行的交互回调。**必须是一个稳定引用**（`ClipboardWorkspace` 里的模块级常量），
 * 每次渲染新建一份会让下面的 `memo` 完全失效。所以回调收的是 `clip`/`index`
 * 而不是靠闭包捕获它们。
 */
export type ClipboardRowHandlers = {
  onFocus: (index: number) => void;
  onToggle: (clip: ClipItem, index: number) => void;
  onAction: (clip: ClipItem, index: number, action: Action, actionIndex: number) => void;
};

type Props = {
  clip: ClipItem;
  index: number;
  focused: boolean;
  /** 焦点落在第几个行内动作上；`-1` 是行本体，未获焦的行恒为 `-1`。 */
  focusedAction: number;
  expanded: boolean;
  favoriteMode: boolean;
  /** 当前语言：`t()` 的结果不是 props 的函数，切语言时靠它让 `memo` 失效。 */
  locale: string;
  handlers: ClipboardRowHandlers;
};

/**
 * 一行剪贴板记录。
 *
 * **props 是拍扁的标量而不是整个 snapshot，而且外面包了 `memo`。** 上下移动焦点、
 * 展开动作、鼠标划过列表都会产生一份新 snapshot，把它整个传进来等于每次按键都要把
 * 全部 30 行连着每行 5 个 lucide 图标重新 reconcile 一遍；拍扁之后一次焦点移动只重渲
 * 两行（失焦的那行和获焦的那行）。加字段时记得它必须是标量，别把 snapshot 引回来。
 */
export const ClipboardRow = memo(function ClipboardRow({
  clip,
  index,
  focused,
  focusedAction,
  expanded,
  favoriteMode,
  handlers,
}: Props) {
  const rowRef = useRef<HTMLDivElement>(null);
  const [nearViewport, setNearViewport] = useState(false);
  const [imageBase64, setImageBase64] = useState<string | null>(null);
  // 行里不显示内容类型（既不是 badge 也不进 meta）：后端 content_type 只有
  // text/html/image 三档，而右侧预览按内容嗅探，同一条会一边写 HTML 一边写 YAML。
  // 类型统一由 preview/classify.js 判定，只显示在预览面板的 badge 上。
  // 富文本没有纯文本副本时仍给一句占位——那是"内容为空"的提示，不是类型标签。
  const previewText = clip.content_type === "html"
    ? (clip.text_content || t("preview.richText"))
    : (clip.text_content || "");

  useEffect(() => {
    if (clip.content_type !== "image") {
      setNearViewport(false);
      return;
    }
    const row = rowRef.current;
    if (!row || typeof IntersectionObserver === "undefined") {
      // 旧 WebKit 没有 IntersectionObserver 时保持原有行为，功能不能因优化而消失。
      setNearViewport(true);
      return;
    }
    const observer = new IntersectionObserver(
      ([entry]) => setNearViewport(entry?.isIntersecting === true),
      { root: row.closest(".clip-list"), rootMargin: "160px 0px" },
    );
    observer.observe(row);
    return () => observer.disconnect();
  }, [clip.content_type, clip.id]);

  useEffect(() => {
    let cancelled = false;
    if (clip.content_type !== "image" || !nearViewport) {
      setImageBase64(null);
      return;
    }
    // 缩略图而不是原图：这一格是 48×48，取原图等于为了画 48 px 把几 MB 的 PNG
    // 送进 webview 再全尺寸解码一次。并且只保留视口附近的缩略图：最大历史量允许
    // 10,000 条，滚过的每一张都常驻 base64 + 解码纹理会让面板内存只增不减。
    getClipThumbnail(clip.id)
      .then((value) => !cancelled && setImageBase64(value))
      .catch(() => undefined);
    return () => { cancelled = true; };
  }, [clip.content_type, clip.id, nearViewport]);

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
      ref={rowRef}
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
      onPointerMove={() => handlers.onFocus(index)}
      onClick={() => handlers.onAction(clip, index, "copy", -1)}
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
              focusedAction === actionIndex ? "focused" : "",
            ].filter(Boolean).join(" ")}
            aria-label={action.label}
            title={action.label}
            onClick={(event) => {
              event.stopPropagation();
              handlers.onAction(clip, index, action.key, actionIndex);
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
          handlers.onToggle(clip, index);
        }}
      >
        <Ellipsis size={16} />
      </button>
    </div>
  );
});
