import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { drawScene } from "../annotation/canvasRenderer";
import { DEFAULT_IMAGE_ADJUSTMENTS } from "../annotation/imageAdjustments";
import { exportPngBase64 } from "../annotation/pngPipeline";
import type { Annotation, Tool } from "../annotation/types";
import { useCanvasInteractions } from "../annotation/useCanvasInteractions";
import { useHistory } from "../annotation/useHistory";
import { DEFAULT_COLOR, DEFAULT_STROKE } from "../capture-overlay/tools";

/**
 * 贴图上的画布：标注状态、渲染与导出。
 *
 * 用的是和截图覆盖层完全同一套标注基建（`annotation/`）——工具、渲染、撤销栈、导出
 * 一个字都没重写，所以两处画出来的东西一模一样，加工具也只加一处。
 *
 * **坐标空间是图片像素**，和覆盖层一致：标注存在像素空间里，导出时不必二次换算，
 * 缩放显示器上也不会错位。屏上尺寸由 `scale`（CSS 尺寸 ÷ 图片像素）折算。
 *
 * **不改贴图条目**。条目里那张始终是原图，`copy_pin`/`save_pin` 一直交付它；
 * 画完的东西要留下来得显式存盘（`save_pin_canvas`）。画布关掉不丢标注，
 * 用户可以再打开接着画——丢弃是显式动作。
 */
export function usePinCanvas(params: {
  /** 贴图的底图。没加载完时传 `null`，那时画布不渲染。 */
  image: HTMLImageElement | null;
  /** 底图像素尺寸。 */
  pixelWidth: number;
  pixelHeight: number;
  /** 内容区的 CSS 尺寸，用来把像素空间折成屏上尺寸。 */
  cssWidth: number;
  cssHeight: number;
  /** 画布是否开着。关着时不挂交互、不渲染。 */
  open: boolean;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(params.image);
  imageRef.current = params.image;

  const history = useHistory<Annotation[]>([]);
  const annotations = history.value;
  const [tool, setTool] = useState<Tool>("pen");
  const [color, setColor] = useState(DEFAULT_COLOR);
  const [stroke, setStroke] = useState(DEFAULT_STROKE);
  const [text, setText] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // 屏上一个 CSS 像素对应几个图片像素。标注坐标是图片像素，交互层拿它换算指针位置。
  const scale = useMemo(
    () => (params.pixelWidth > 0 ? params.cssWidth / params.pixelWidth : 1),
    [params.cssWidth, params.pixelWidth],
  );

  const canvas = useCanvasInteractions({
    imageRef,
    canvasRef,
    scale,
    // 贴图没有"裁剪"这件事（整张图就是画布），但交互层要求给一个工具名；
    // `crop` 只会写下面那两个被丢弃的回调，是最无害的落点——和覆盖层同一处理。
    tool: tool === "crop" ? "crop" : tool,
    color,
    size: stroke,
    text,
    annotations,
    selection: null,
    setSelection: () => {},
    onSelect: (annotation) => setSelectedId(annotation?.id ?? null),
    commitAnnotations: history.commit,
  });

  // 交互层给的是"拖拽草稿"，渲染层要的是里面那个标注（`crop` 草稿没有标注，贴图也用不到）。
  const draft = canvas.draft && "annotation" in canvas.draft ? canvas.draft.annotation : null;

  useEffect(() => {
    const target = canvasRef.current;
    const image = imageRef.current;
    if (!params.open || !target || !image) return;
    drawScene(
      target,
      image,
      {
        width: params.cssWidth,
        height: params.cssHeight,
        fitScale: scale,
        zoom: 1,
        scale,
      },
      annotations,
      draft,
      DEFAULT_IMAGE_ADJUSTMENTS,
      selectedId,
    );
  }, [
    annotations,
    draft,
    params.cssHeight,
    params.cssWidth,
    params.image,
    params.open,
    scale,
    selectedId,
  ]);

  /** 画过东西了吗？关窗时据此决定要不要问"保存？"。 */
  const dirty = annotations.length > 0;

  /** 导出"底图 + 标注"的 PNG（图片原始分辨率，不受屏上缩放影响）。 */
  const exportPng = useCallback(async (): Promise<string> => {
    const image = imageRef.current;
    if (!image) throw new Error("Pin image is not ready");
    return exportPngBase64(
      image,
      { x: 0, y: 0, width: params.pixelWidth, height: params.pixelHeight },
      annotations,
      DEFAULT_IMAGE_ADJUSTMENTS,
    );
  }, [annotations, params.pixelHeight, params.pixelWidth]);

  const deleteSelected = useCallback(() => {
    if (!selectedId) return;
    history.commit((items) => items.filter((item) => item.id !== selectedId));
    setSelectedId(null);
  }, [history, selectedId]);

  // **返回值必须是稳定引用。** 这个 hook 的结果会进 `App.tsx` 里 keydown effect 的
  // 依赖数组，而滚轮缩放的每一帧都会重渲染贴图——返回裸对象字面量的话，那个 effect
  // 每帧都要 remove/addEventListener 一次，`requestClose` / `saveCanvas` 也跟着每帧重建。
  // 缩放是这个窗口最高频的交互，那条路专门优化过（`update_pin` 的在飞合并），
  // 不能在这里又加一份每帧开销。
  return useMemo(
    () => ({
      canvasRef,
      tool,
      setTool,
      color,
      setColor,
      stroke,
      setStroke,
      text,
      setText,
      hasSelectedObject: selectedId !== null,
      canUndo: history.canUndo,
      canRedo: history.canRedo,
      undo: history.undo,
      redo: history.redo,
      dirty,
      exportPng,
      deleteSelected,
      onPointerDown: canvas.onPointerDown,
      onPointerMove: canvas.onPointerMove,
      onPointerUp: canvas.onPointerUp,
    }),
    [
      canvas.onPointerDown,
      canvas.onPointerMove,
      canvas.onPointerUp,
      color,
      deleteSelected,
      dirty,
      exportPng,
      history.canRedo,
      history.canUndo,
      history.redo,
      history.undo,
      selectedId,
      stroke,
      text,
      tool,
    ],
  );
}
