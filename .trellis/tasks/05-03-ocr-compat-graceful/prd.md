# OCR 动态链接兼容性修复 + 依赖缺失友好降级

## Goal

v0.1.11 引入 `leptess = "0.14"` 后，OCR 功能动态链接了 libtesseract 和 libleptonica。CI 在 ubuntu-22.04 上构建产出的二进制链接 `libtesseract.so.4` + `liblept.so.5`，而用户系统（Ubuntu 24.04/26.04）只有 `libtesseract.so.5` + `libleptonica.so.6`，SONAME 不匹配导致应用启动直接失败。

需要：
1. 确保 CI 构建产物兼容目标系统
2. 当 OCR 依赖缺失时应用仍能启动（友好降级，OCR 不可用但不崩溃）
3. 建立预防机制避免类似问题

## What I already know

- `leptess` → `tesseract-sys` → 通过 `pkg-config` 检测 libtesseract，`cargo:rustc-link-lib=tesseract`（无 static/dylib 前缀，链接器自选）
- Ubuntu 22.04: `libtesseract.so.4` + `liblept.so.5`
- Ubuntu 24.04: `libtesseract.so.5` + `libleptonica.so.6`
- Ubuntu 26.04: 同 24.04
- 本地有静态库 `.a` 时链接器会选静态链接（本地编译无 .so 依赖）
- CI 上 `-dev` 包可能不含 `.a`，默认动态链接
- 已将 CI 升级到 ubuntu-24.04 matrix（build.yml + release.yml），但仍需验证
- `ocr.rs` 中 `leptess::LepTess` 在全局 `Mutex<Option<LepTess>>` 中 lazy init
- 但 `leptess` crate 是**编译时链接**的，不是运行时 dlopen，所以即使 lazy init 也无法避免启动时 dynamic linker 报错

## Requirements

1. **CI 构建兼容**：确保 release deb 在 Ubuntu 24.04+ 上可运行
2. **运行时友好降级**：OCR 依赖缺失时应用正常启动，OCR 功能标记为不可用，前端显示提示
3. **CI 多版本构建**：已实现 ubuntu-22/24 matrix（已推送）
4. **前端提示**：OCR 不可用时在预览面板显示"OCR 需要安装 libtesseract5"提示

## Acceptance Criteria

- [ ] deb 安装后在 Ubuntu 24.04/26.04 上可启动
- [ ] 缺少 libtesseract 时应用仍可启动，OCR 按钮/功能显示不可用提示
- [ ] CI check 在 ubuntu-24.04 上通过
- [ ] cargo clippy 通过

## Technical Notes

### 方案：运行时 dlopen 替代编译时链接

将 `leptess` 从编译时依赖改为**运行时 dlopen**（通过 `libloading` crate）：
- 优点：二进制不再 NEEDED libtesseract.so，启动不受影响
- 缺点：实现复杂度高，需要手写 C API 绑定

### 方案 B（推荐）：Cargo feature gate

将 `leptess` 放到 optional feature 下：
- CI ubuntu-22 不启用 OCR feature
- CI ubuntu-24 启用 OCR feature
- 无 OCR 的版本用 stub 函数返回 Err

### 方案 C（最简）：仅升级 CI + 文档说明

保持现状，CI 已升级到 24.04，deb depends 声明正确。
用户通过 `apt install ./clippy.deb` 安装时会自动拉取依赖。

## Out of Scope

- 静态链接 tesseract 到二进制（引入巨大依赖树）
- Windows/macOS 支持
