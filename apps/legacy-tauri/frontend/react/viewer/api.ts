import {
  getViewerSettings, getViewerPayload, getViewerImageUrl, viewerReady, closeImageViewer,
  recognizeViewer, detectViewerCodes, translateViewer, sampleViewerColor,
  copyViewerImage, saveViewerImage, pinViewerImage, copyViewerText, onCurrentWindowCloseRequested,
  getViewerFullscreen, setViewerFullscreen, minimizeImageViewer, startViewerDrag, onViewerWindowChanged,
} from "../../js/api.ts";

/** 生产与开发宿主共享一个服务合同，所有原生访问仍在根 api.ts。 */
export const viewerApi = {
  get: getViewerPayload, config: getViewerSettings, imageUrl: getViewerImageUrl,
  ready: viewerReady, close: closeImageViewer, onCloseRequested: onCurrentWindowCloseRequested,
  recognize: recognizeViewer, scan: detectViewerCodes, translate: translateViewer,
  sample: sampleViewerColor, copyImage: copyViewerImage, save: saveViewerImage,
  pin: pinViewerImage, copyText: copyViewerText,
  getFullscreen: getViewerFullscreen, setFullscreen: setViewerFullscreen,
  minimize: minimizeImageViewer, startDrag: startViewerDrag, onWindowChanged: onViewerWindowChanged,
};
export type ViewerServices = typeof viewerApi;
