// 一个真实 iframe viewport 保持 fullscreen App 的 clientX/clientY 原点为零。
if (!import.meta.env.DEV) throw new Error("Development review is unavailable in production");
const frame = document.querySelector("iframe");
frame.src = `./capture-frame.html${location.search}`;
window.addEventListener("message", event => {
  if (event.source === frame.contentWindow && event.origin === location.origin
    && event.data?.type === "capture-review-status" && typeof event.data.status === "string") {
    document.querySelector('[role="status"]').textContent = event.data.status;
  }
});
