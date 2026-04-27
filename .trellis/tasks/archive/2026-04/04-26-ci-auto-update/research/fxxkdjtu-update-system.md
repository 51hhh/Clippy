# fxxkDJTU 自动更新系统参考

## 技术栈
- Rust: `tauri-plugin-updater = "2"`
- 前端: `@tauri-apps/plugin-updater ^2.10.1`
- CI: `tauri-apps/tauri-action`, `includeUpdaterJson: true`

## tauri.conf.json 配置
```json
"plugins": {
  "updater": {
    "pubkey": "<minisign public key>",
    "endpoints": ["https://github.com/51hhh/fxxkDJTU/releases/latest/download/latest.json"]
  }
},
"bundle": { "createUpdaterArtifacts": true }
```

## capabilities/default.json
需添加 `"updater:default"` 权限。

## 更新检查流程
1. `check()` from `@tauri-apps/plugin-updater` → 请求 latest.json
2. 比较版本 → 有新版本时弹窗
3. `localStorage.skipped_update_version` 跳过记忆
4. `downloadAndInstall()` + 进度回调（Started/Progress/Finished）
5. deb 不支持 → fallback 到 GitHub Release 页面

## Changelog 格式
```markdown
## v0.1.10
### ✨ 新功能
- 功能描述
### 🐞 修复
- 修复描述
```

## CI Release 流水线
tag v*.*.* → check-version → 并行构建 → update-release（awk 提取 changelog → gh release edit）

## 自动更新支持矩阵
| 格式 | 自动更新 |
|------|---------|
| AppImage | ✅ |
| deb | ❌ → 手动回退 |
