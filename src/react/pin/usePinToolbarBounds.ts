import { useCallback, useEffect, useRef, useState } from "react";
import { fullWindowBounds, type ToolbarBounds } from "../shared/toolbarPlacement";
import { pinApi } from "./api";

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
        setBounds(next.width > 0 && next.height > 0 ? next : null);
      })
      .catch((reason) => {
        // 边界查不到只影响工具条落点，不该打扰用户，也不该让贴图看起来出错了。
        console.debug("贴图工具条边界查询失败", reason);
        setBounds(null);
      })
      .finally(() => {
        inFlight.current = false;
        if (again.current) {
          again.current = false;
          refresh();
        }
      });
  }, [label]);

  // 尺寸变化（滚轮缩放会改窗口尺寸）之后重问一次。`viewport` 本身就是 resize 驱动的，
  // 所以这一条同时覆盖了"缩放完"和"窗口被合成器改了尺寸"。
  useEffect(() => {
    refresh();
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
    window.addEventListener("pointerup", refresh);
    window.addEventListener("focus", refresh);
    return () => {
      window.removeEventListener("pointerup", refresh);
      window.removeEventListener("focus", refresh);
    };
  }, [refresh]);

  return bounds ?? fullWindowBounds(viewport);
}
