/**
 * preview/renderers.js — 预览渲染功能的统一工厂入口
 */

import { createCodeRenderers } from "./code-renderers.js";
import { createMetadataRenderers } from "./metadata-renderers.js";
import { createFormatRenderers } from "./format-renderers.js";
import { createEncryptedRenderer } from "./encrypted-renderer.js";
import { createContentRenderers } from "./content-renderers.js";

export function createPreviewRenderers(context) {
  return {
    ...createCodeRenderers(context),
    ...createMetadataRenderers(context),
    ...createFormatRenderers(context),
    ...createEncryptedRenderer(context),
    ...createContentRenderers(context),
  };
}
