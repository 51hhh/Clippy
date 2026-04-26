---
description: "Use when writing or modifying frontend JavaScript/CSS/HTML in src/. Covers vanilla JS component patterns, IPC via api.js, i18n, theme system, and XSS prevention."
applyTo: "src/js/**,src/styles/**,src/*.html"
---
# 前端约定

## 模块架构
- 纯 vanilla HTML/CSS/JS + ES Module `<script type="module">`
- **只有 `api.js` 允许直接访问 `window.__TAURI__`**，其他模块通过 `api.js` 导出间接调用
- settings.js 是例外：独立窗口，可直接 `invoke`

## XSS 防护
所有用户内容通过 `textContent` 写入 DOM，**禁止 `innerHTML`**。

## IPC 事件
监听: `clip-added`, `clip-removed`, `config-changed`, `shortcut-register-failed`  
命令: `get_clips`, `delete_clip`, `toggle_favorite`, `clear_history`, `select_clip`, `get_config`, `update_config`, `update_shortcut`, `check_shortcut_conflict`, `pause_shortcuts`, `resume_shortcuts`

## 国际化
- `i18n/i18n.js` 提供翻译 API，支持 `{n}` 占位符插值
- 翻译文件: `i18n/en.json`, `i18n/zh-CN.json`

## 主题系统
- 6 个 CSS 主题通过 CSS 自定义属性切换
- 定义在 `styles/themes.css`

## 测试
- Vitest + jsdom 环境
- 测试文件在 `src/tests/`
- 运行: `cd src && npx vitest run`

## 语言
- UI 文本使用**英文**
- 代码注释使用**中文**
