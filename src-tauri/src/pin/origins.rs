use super::model::PinOrigin;
use std::collections::VecDeque;
use std::sync::Mutex;

/// 记多少条。"截图 → 复制 → 从历史里 Pin 回来"是个很短的链路，十几条足够；
/// 这张表纯属锦上添花，为它长期占内存不值得。
const CAPACITY: usize = 16;

/// 记住"我们自己截下来、复制进剪贴板的图"原本在屏幕上的哪一块。
///
/// 用户可以先复制截图、过一会儿再从剪贴板历史里把它 Pin 出来。到那一刻后端手里只有
/// 一串 PNG 字节，无从知道它当初框的是屏幕的哪个位置，于是贴图只能落在光标旁边。
/// 所以在复制的那一刻按图像内容把矩形登记下来，Pin 的时候再按内容查回去。
///
/// 键是**解码后像素**的哈希，不是 PNG 字节的哈希：图片进出剪贴板会被 arboard 拆成裸
/// RGBA、再由监听线程重新编码成 PNG，字节流根本不稳定（编码器参数、chunk 顺序都可能变），
/// 只有像素是稳定的。查不到就是查不到——退回默认摆放，绝不能因此让 Pin 失败。
#[derive(Default)]
pub(crate) struct PinOriginRegistry {
    /// 先进先出，超过 `CAPACITY` 丢最旧的。锁只在登记/查询的瞬间持有。
    entries: Mutex<VecDeque<Entry>>,
}

struct Entry {
    key: String,
    /// 尺寸单独留一份，用来在 `lookup` 里先做一次廉价预筛（见那里的注释）。
    width: u32,
    height: u32,
    origin: PinOrigin,
}

/// 一张图的像素指纹。
///
/// 单独成一个类型是为了让调用方能"先算好、等剪贴板真的写成功了再落表"：算指纹要的是
/// 解码后的 RGBA（一张全屏图 16 MB），而那份 RGBA 紧接着就要交给 arboard，
/// 为了推迟登记而把它整份拷一遍不值得。
pub(crate) struct PinFingerprint {
    key: String,
    width: u32,
    height: u32,
}

impl PinFingerprint {
    /// 从**已经解码好**的像素算指纹。
    ///
    /// 只收 RGBA 而不收 PNG：这条路上的调用方（复制到剪贴板）刚刚为了写剪贴板解过一次
    /// PNG，像素就在手边，再解一遍是白花几十毫秒。
    pub(crate) fn of(width: u32, height: u32, rgba: &[u8]) -> Self {
        Self {
            key: pixel_key(width, height, rgba),
            width,
            height,
        }
    }
}

impl PinOriginRegistry {
    pub(crate) fn remember(&self, fingerprint: PinFingerprint, origin: PinOrigin) {
        let Some(origin) = origin.sanitized() else {
            return;
        };
        let PinFingerprint { key, width, height } = fingerprint;
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        entries.retain(|entry| entry.key != key);
        entries.push_back(Entry {
            key,
            width,
            height,
            origin,
        });
        while entries.len() > CAPACITY {
            entries.pop_front();
        }
    }

    /// 查这串 PNG 的来源矩形。
    ///
    /// 先读 PNG 文件头里的宽高做预筛：表里没有同尺寸的条目就不可能命中，而读一个头
    /// 比解一整张全屏图便宜几个数量级。绝大多数 Pin 面对的都是与截图无关的图片，
    /// 这一层挡住的就是它们（表空时同样在这里返回）。
    pub(crate) fn lookup(&self, png: &[u8]) -> Option<PinOrigin> {
        let (width, height) = crate::screenshot::png_dimensions(png).ok()?;
        {
            let entries = self.entries.lock().ok()?;
            if !entries
                .iter()
                .any(|entry| entry.width == width && entry.height == height)
            {
                return None;
            }
        }
        let image = image::load_from_memory(png).ok()?.into_rgba8();
        let key = pixel_key(image.width(), image.height(), image.as_raw());
        let entries = self.entries.lock().ok()?;
        entries
            .iter()
            .rev()
            .find(|entry| entry.key == key)
            .map(|entry| entry.origin)
    }
}

/// 图像像素的指纹：尺寸 + 全部 RGBA 字节的 sha256。尺寸也进哈希，避免同样的字节
/// 按不同宽高解读时撞键。逐段喂给 hasher 而不是先拼一个 payload——全屏图的 RGBA
/// 有 16 MB，为了在前面加 8 个字节而整份拷贝一遍不值得。
fn pixel_key(width: u32, height: u32, rgba: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(width.to_le_bytes());
    hasher.update(height.to_le_bytes());
    hasher.update(rgba);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(pixel: [u8; 4]) -> Vec<u8> {
        crate::screenshot::encode_png(&pixel, 1, 1).unwrap()
    }

    /// 生产代码走的是"剪贴板路径上已经有 RGBA"那条入口，测试里从 PNG 现解一次。
    fn remember(registry: &PinOriginRegistry, png: &[u8], origin: PinOrigin) {
        let image = image::load_from_memory(png).unwrap().into_rgba8();
        registry.remember(
            PinFingerprint::of(image.width(), image.height(), image.as_raw()),
            origin,
        );
    }

    fn origin(x: f64) -> PinOrigin {
        PinOrigin {
            x,
            y: 20.0,
            width: 300.0,
            height: 200.0,
        }
    }

    #[test]
    fn origin_is_found_again_by_pixels_not_by_png_bytes() {
        let registry = PinOriginRegistry::default();
        let red = png([255, 0, 0, 255]);
        remember(&registry, &red, origin(10.0));
        // 重新编码一遍模拟"经剪贴板往返"：字节可能不同，像素一样就该命中。
        let reencoded = crate::screenshot::encode_png(
            image::load_from_memory(&red).unwrap().to_rgba8().as_raw(),
            1,
            1,
        )
        .unwrap();
        assert_eq!(registry.lookup(&reencoded), Some(origin(10.0)));
        assert_eq!(registry.lookup(&png([0, 255, 0, 255])), None);
    }

    #[test]
    fn empty_registry_and_broken_images_never_panic_or_match() {
        let registry = PinOriginRegistry::default();
        assert_eq!(registry.lookup(&png([1, 2, 3, 4])), None);
        assert_eq!(registry.lookup(b"not a png"), None);
        // 非法矩形不该进表，否则会一路污染窗口几何。
        remember(
            &registry,
            &png([1, 2, 3, 4]),
            PinOrigin {
                x: f64::NAN,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
        );
        assert_eq!(registry.lookup(&png([1, 2, 3, 4])), None);
    }

    /// 尺寸预筛只能挡住不可能命中的图，不许把真命中的挡掉：同尺寸不同像素照样要判不中，
    /// 不同尺寸的图直接被头部预筛拦下，同一张图无论走哪条都要认得出来。
    #[test]
    fn the_size_prefilter_never_hides_a_real_match() {
        let registry = PinOriginRegistry::default();
        let wide = crate::screenshot::encode_png(&[1, 2, 3, 255, 4, 5, 6, 255], 2, 1).unwrap();
        remember(&registry, &wide, origin(40.0));

        // 1×1 与表里唯一的 2×1 尺寸不符，连解码都不必做
        assert_eq!(registry.lookup(&png([1, 2, 3, 255])), None);
        assert_eq!(registry.lookup(&wide), Some(origin(40.0)));

        // 同尺寸但像素不同：预筛放过，全量指纹必须判不中
        let other = crate::screenshot::encode_png(&[9, 9, 9, 255, 9, 9, 9, 255], 2, 1).unwrap();
        assert_eq!(registry.lookup(&other), None);
    }

    #[test]
    fn oldest_entries_are_dropped_and_repeats_are_deduplicated() {
        let registry = PinOriginRegistry::default();
        let first = png([9, 9, 9, 255]);
        remember(&registry, &first, origin(1.0));
        for index in 0..CAPACITY as u8 {
            remember(
                &registry,
                &png([index, 0, 0, 255]),
                origin(index as f64 + 100.0),
            );
        }
        // 第一条被挤出去了
        assert_eq!(registry.lookup(&first), None);
        // 同一张图重复登记只留最后一次的位置
        let repeat = png([7, 7, 7, 255]);
        remember(&registry, &repeat, origin(2.0));
        remember(&registry, &repeat, origin(3.0));
        assert_eq!(registry.lookup(&repeat), Some(origin(3.0)));
        assert_eq!(registry.entries.lock().unwrap().len(), CAPACITY);
    }
}
