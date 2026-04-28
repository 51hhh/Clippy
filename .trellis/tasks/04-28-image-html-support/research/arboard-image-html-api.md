# Research: arboard Image & HTML Clipboard API

## 1. arboard 3.x Image API

### `get_image()` — 读取剪贴板图片

```rust
use arboard::Clipboard;

let mut clipboard = Clipboard::new().unwrap();
let image = clipboard.get_image().unwrap();
// image: arboard::ImageData<'static>
// Fields: width: usize, height: usize, bytes: Cow<'static, [u8]>
// Format: RGBA raw bytes (4 bytes per pixel), row-major order
```

`ImageData` 包含原始 RGBA 像素数据。总字节数 = `width * height * 4`。

### `set_image()` — 写入图片到剪贴板

```rust
use arboard::{Clipboard, ImageData};
use std::borrow::Cow;

let mut clipboard = Clipboard::new().unwrap();
let img = ImageData {
    width: 100,
    height: 100,
    bytes: Cow::Owned(rgba_bytes),
};
clipboard.set_image(img).unwrap();
```

### PNG 编码存储 (使用 `image` crate)

```rust
use image::{ImageBuffer, RgbaImage};
use std::io::Cursor;

fn encode_to_png(img: &arboard::ImageData) -> Vec<u8> {
    let rgba: RgbaImage = ImageBuffer::from_raw(
        img.width as u32,
        img.height as u32,
        img.bytes.to_vec(),
    ).expect("Invalid image dimensions");
    
    let mut buf = Cursor::new(Vec::new());
    rgba.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}
```

### PNG 解码 (回写 set_image)

```rust
fn decode_from_png(png_bytes: &[u8]) -> arboard::ImageData<'static> {
    let img = image::load_from_memory(png_bytes).unwrap().to_rgba8();
    let (w, h) = img.dimensions();
    arboard::ImageData {
        width: w as usize,
        height: h as usize,
        bytes: Cow::Owned(img.into_raw()),
    }
}
```

**依赖**: `image = "0.25"`

## 2. arboard HTML 支持

### 现状 (arboard 3.x)

arboard **不支持** `get_html()` / `set_html()`。公开 API 只有：
- `get_text()` / `set_text()` — 纯文本
- `get_image()` / `set_image()` — 图片

### Linux HTML 剪贴板替代方案

**方案 A: `xclip` / `wl-paste` CLI** (MVP 最简)
```rust
// X11 — 读取 HTML
fn get_html_x11() -> Option<String> {
    let output = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "text/html", "-o"])
        .output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

// Wayland — 读取 HTML
fn get_html_wayland() -> Option<String> {
    let output = Command::new("wl-paste")
        .args(["--type", "text/html"])
        .output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}
```

**建议**: Linux-only 项目用 CLI 包装最实用，通过 `$XDG_SESSION_TYPE` 检测会话类型。

## 3. 图片 SQLite 存储

- **编码为 PNG** 后存储（无损、截图典型 200KB-2MB）
- **大小上限**: 单张图片 ~10MB
- **SHA-256 哈希**: 在 RGBA 原始字节上计算（PNG 编码前），保证去重一致性
- **缩略图独立存储**: 避免列表查询时解码全尺寸图片

### 现有 Schema 已满足需求
```sql
clips 表已有: image_data BLOB, content_type TEXT
-- 可额外增加 thumbnail_data BLOB 列
```

## 4. 缩略图生成

```rust
use image::{DynamicImage, imageops::FilterType};

fn generate_thumbnail(png_bytes: &[u8], max_width: u32) -> Vec<u8> {
    let img = image::load_from_memory(png_bytes).unwrap();
    let (w, h) = (img.width(), img.height());
    if w <= max_width {
        return png_bytes.to_vec();
    }
    let new_height = (h as f64 * max_width as f64 / w as f64) as u32;
    let thumb = img.resize(max_width, new_height, FilterType::Lanczos3);
    let mut buf = Cursor::new(Vec::new());
    thumb.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}
```

建议缩略图宽度: 200px

## 5. 前端显示方案

### 方案 A: Base64 Data URL（推荐 MVP）

- 缩略图作为 `get_clips()` 响应的一部分返回（base64 string 字段）
- 前端 `img.src = dataUrl` 直接显示
- 缩略图小（5-20KB），base64 开销可接受

### 方案 B: Tauri Asset Protocol（大图预览）

- 注册自定义协议 `clip-image://`
- 大图按需加载，无 base64 开销

**MVP 推荐**: 缩略图和全尺寸图都用 base64 data URL，后续有性能问题再迁移 asset protocol。

## 注意事项

- `get_image()` 可能返回 `Err`（剪贴板无图片）— 监听循环需优雅处理
- PNG 编码/缩略图生成有 CPU 开销 — 在 watcher 线程中同步处理即可
- 4K 截图 = ~33MB RGBA 原始数据，需注意内存
- HTML 依赖 `xclip`/`wl-clipboard` — 应为软依赖，缺失时回退到纯文本
- `image` crate 增加编译时间 (~30s 首次)
