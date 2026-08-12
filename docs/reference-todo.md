# 参考项目借鉴 TODO

> 从 translator/flashot 提取的可借鉴实现，按优先级排列。

## P0: Pin 窗口 Wayland 置顶评估
- [ ] 调研 Wayland 小窗置顶能力，不直接复用截图 overlay 的四边锚定逻辑
- [ ] 如引入 layer-shell，必须保留 pin 窗口尺寸/位置并补充手动验证
- [x] X11/通用路径保持 always_on_top，前端 setAlwaysOnTop 作为兜底
- 来源: flashot overlay_window.rs（仅参考置顶思路，不复用全屏 overlay 形态）

## P1: Pin 窗口管理器 (PinManager)
- [x] PinManager (`std::sync::Mutex<HashMap>`，避免为单一锁引入额外依赖)
- [x] 图片数据由 PinEntry 内存所有权管理，不生成 app_cache_dir 临时文件
- [x] 窗口销毁时从 PinManager 自动移除条目 (`on_window_event`)
- [x] 支持缩放、透明度和锁定状态跟踪
- [x] 剪贴板与截图 Pin 统一使用 AppState PinManager
- 来源: flashot pin_mgr.rs

## P2: 剪贴板写入重试
- [x] clipboard 写入失败时 sleep 30ms + 重试一次
- [x] 记录 warn 日志
- 来源: flashot clipboard.rs

## P3: 图像调整 (brightness/contrast/saturation)
- [x] ImageAdjustments 结构体 (grayscale, brightness, contrast, saturation)
- [ ] 纯 CPU 逐像素处理 (f32 运算)
- [x] 前端归一化/filter 单元测试覆盖
- 来源: flashot image_adjust.rs

## P3.5: 截图编辑器
- [x] Linux 截图 fallback：xcap + Wayland/wlroots + Portal + GNOME Shell
- [x] React/TS 截图编辑功能岛
- [x] 区域选择、画笔、矩形、箭头、文字、复制/保存/贴图
- [ ] 滚动截图
- [x] 窗口候选探测与鼠标位置智能命中
- 来源: flashot capture/, overlay/, annotation/

## P4: 错误类型化
- [ ] 用 thiserror 定义 ClippyError 枚举
- [ ] command 层 .map_err 保持 String 但内部结构化
- 来源: translator error.rs

## P5: 配置版本迁移
- [x] AppConfig 加 version 字段
- [x] serde default + 向后兼容
- 来源: translator config.rs, flashot settings_store.rs

## P6: Release profile 优化
- [x] opt-level = "s"
- [x] 使用 lto = "fat" (translator 用法)
- 来源: translator Cargo.toml

## P7: ci-local.sh 增强
- [x] 添加 DOM/Xvfb smoke 与 Linux 全目标编译检查
- [ ] require_cmd 检查
- 来源: flashot scripts/ci-local.sh
