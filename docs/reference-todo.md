# 参考项目借鉴 TODO

> 从 translator/flashot 提取的可借鉴实现，按优先级排列。

## P0: Pin 窗口 Wayland 置顶评估
- [ ] 调研 Wayland 小窗置顶能力，不直接复用截图 overlay 的四边锚定逻辑
- [ ] 如引入 layer-shell，必须保留 pin 窗口尺寸/位置并补充手动验证
- [x] X11/通用路径保持 always_on_top，前端 setAlwaysOnTop 作为兜底
- 来源: flashot overlay_window.rs（仅参考置顶思路，不复用全屏 overlay 形态）

## P1: Pin 窗口管理器 (PinManager)
- [ ] PinManager (parking_lot::Mutex<HashMap>)
- [ ] Pin 图片保存到 app_cache_dir/pins/
- [ ] 窗口销毁时自动清理文件 (on_window_event)
- [ ] 支持缩放状态跟踪
- 来源: flashot pin_mgr.rs

## P2: 剪贴板写入重试
- [x] clipboard 写入失败时 sleep 30ms + 重试一次
- [x] 记录 warn 日志
- 来源: flashot clipboard.rs

## P3: 图像调整 (brightness/contrast/saturation)
- [ ] ImageAdjustments 结构体 (grayscale, brightness, contrast, saturation)
- [ ] 纯 CPU 逐像素处理 (f32 运算)
- [ ] 单元测试覆盖
- 来源: flashot image_adjust.rs

## P4: 错误类型化
- [ ] 用 thiserror 定义 ClippyError 枚举
- [ ] command 层 .map_err 保持 String 但内部结构化
- 来源: translator error.rs

## P5: 配置版本迁移
- [x] AppConfig 加 version 字段
- [x] serde default + 向后兼容
- 来源: translator config.rs, flashot settings_store.rs

## P6: Release profile 优化
- [ ] opt-level = "s" 已有
- [x] 使用 lto = "fat" (translator 用法)
- 来源: translator Cargo.toml

## P7: ci-local.sh 增强
- [ ] 添加 smoke check (Linux platform API 编译验证)
- [ ] require_cmd 检查
- 来源: flashot scripts/ci-local.sh
