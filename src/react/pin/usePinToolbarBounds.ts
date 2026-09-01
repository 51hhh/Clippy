import { useCallback, useEffect, useRef, useState } from "react";
import { fullWindowBounds, type ToolbarBounds } from "../shared/toolbarPlacement";
import { isToolbarDragging } from "../shared/useToolbarDrag";
import { pinApi } from "./api";

/**
 * 尺寸停止变化多久之后才重问边界。
 *
 * 取值比一格滚轮之间的间隔宽松（滚轮连续滚动时事件间隔通常几十毫秒），
 * 又短到用户察觉不出工具条"晚了一下才归位"。
 */
const RESIZE_SETTLE_MS = 180;

/**
 * 工具条能待的范围：贴图窗口里"还落在屏幕工作区内"的那块。
 *
 * **为什么要问后端。** 前端只有 `window.innerWidth/innerHeight`，而贴图窗口的外框恒等于
 * 「内容 + 12×2 阴影 + 44 控件栏」——它永远给工具条留够了位置，所以拿窗口自己当边界时
 * 右侧候选永远装得下，"超出屏幕自动调整"一次都不会触发。真正会超出的是**窗口在屏幕上**
 * 的位置，而 Wayland 下客户端连自己窗口在哪都不知道，只有合成器知道。
 *
 * **什么时候问。** 只在几何可能变了之后：窗口尺寸变化（缩放）、以及用户拖完窗口。
 * 不轮询——每次查询在 Wayland 下是一趟 D-Bus（本机 1~3 ms），按帧问就是把主循环
 * 当轮询器用。查询是异步的，所以工具条会在拖动结束后一小段时间才归位；
 * 这比"拖动过程中工具条乱跳"好，也比一直在屏外好。
 *
 * 拿不到（后端返回 0 宽高、命令失败、窗口刚关掉）就退回整个窗口，也就是这个功能
 * 存在之前的行为。
 */
export function usePinToolbarBounds(label: string, viewport: { width: number; height: number }) {
  const [bounds, setBounds] = useState<ToolbarBounds | null>(null);
  // 同一时刻只允许一个查询在飞，落地后如果期间又被请求过就再补一次——
  // 和 `update_pin` 的在飞合并同一个套路，避免拖动时堆出一串 D-Bus 请求。
  const inFlight = useRef(false);
  const again = useRef(false);
  // 关掉贴图时查询可能还在飞（异步命令），落地后不能再动已卸载组件的状态。
  const alive = useRef(true);
  useEffect(() => () => {
    alive.current = false;
  }, []);

  const refresh = useCallback(() => {
    if (inFlight.current) {
      again.current = true;
      return;
    }
    inFlight.current = true;
    pinApi
      .toolbarBounds(label)
      .then((next) => {
        // 宽或高为 0 = 后端查不到（窗口刚关掉、扩展还没认出这个窗口）。
        if (alive.current) setBounds(next.width > 0 && next.height > 0 ? next : null);
      })
      .catch((reason) => {
        // 边界查不到只影响工具条落点，不该打扰用户，也不该让贴图看起来出错了。
        console.debug("贴图工具条边界查询失败", reason);
        if (alive.current) setBounds(null);
      })
      .finally(() => {
        inFlight.current = false;
        if (again.current && alive.current) {
          again.current = false;
          refresh();
        }
      });
  }, [label]);

  /**
   * 尺寸变化（滚轮缩放会改窗口尺寸）之后重问一次，但要**等它停下来**。
   *
   * `viewport` 是 `resize` 事件驱动的，而滚轮缩放会让窗口尺寸连续变化——每格滚轮
   * 至少一次 resize。不延迟的话每格都触发一次查询，而单次查询在 Wayland 下是
   * 「25 KB 文件读 + 全串比较 + 一趟 D-Bus + 全窗口 JSON 解析」。在飞合并只挡住了并发，
   * 挡不住这条连续的补发链。
   *
   * 延迟到尺寸稳定之后再问一次就够：缩放期间工具条用的是上一份边界，而缩放本身
   * 不会把窗口挪出屏幕（左上角钉住、往右下长大），边界最多短暂偏一点。
   */
  useEffect(() => {
    const timer = window.setTimeout(refresh, RESIZE_SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [refresh, viewport.width, viewport.height]);

  /**
   * 用户拖完窗口之后重问一次。
   *
   * Wayland 下拖动由合成器接管，我们收不到过程事件，也收不到"拖完了"——只能靠
   * `pointerup` 之后再问。指针在拖动期间被合成器抓走，那次 pointerup 常常送不到
   * WebKit（这正是 `gestures.ts` 里那段"隔一次拖不动"的根因），所以再挂一个
   * `focus`：拖完窗口一定会重新拿到焦点。两条都只是"重问一次"，重复无害。
   */
  useEffect(() => {
    function onPointerUp() {
      // 拖工具条不会挪动窗口，边界没变——那一趟 D-Bus 是白跑的。
      if (isToolbarDragging()) return;
      refresh();
    }
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("focus", refresh);
    return () => {
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("focus", refresh);
    };
  }, [refresh]);

  return bounds ?? fullWindowBounds(viewport);
}
