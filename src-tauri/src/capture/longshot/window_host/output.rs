//! 长截图产物到具体本地输出的适配。

use super::finish::OutputWorkerError;
use super::LongshotOutputArtifact;
use crate::pin::commands::ScreenshotPinCreateError;
use crate::pin::{PinFingerprint, PinOrigin, PinOriginRegistry};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

/// 一次解码同时服务剪贴板写入与来源指纹；只有写入成功才登记可信坐标。
pub(super) fn copy_longshot_artifact<W>(
    artifact: &LongshotOutputArtifact,
    origins: &PinOriginRegistry,
    write_clipboard: W,
) -> Result<(), String>
where
    W: FnOnce(arboard::ImageData<'static>) -> Result<(), String>,
{
    let image = crate::image_io::png_to_clipboard_image(artifact.png.as_slice())?;
    let width = u32::try_from(image.width).map_err(|_| "长截图宽度超出指纹范围".to_string())?;
    let height = u32::try_from(image.height).map_err(|_| "长截图高度超出指纹范围".to_string())?;
    let fingerprint = PinFingerprint::of(width, height, &image.bytes);
    write_clipboard(image)?;
    origins.remember(fingerprint, artifact.origin);
    Ok(())
}

/// 贴图入口接管产物已有的 PNG `Arc` 与后端可信坐标；分类结果直接驱动重试权限。
pub(super) fn pin_longshot_artifact<C>(
    artifact: &LongshotOutputArtifact,
    create_pin: C,
) -> Result<String, OutputWorkerError>
where
    C: FnOnce(Arc<Vec<u8>>, PinOrigin) -> Result<String, ScreenshotPinCreateError>,
{
    match catch_unwind(AssertUnwindSafe(|| {
        create_pin(Arc::clone(&artifact.png), artifact.origin)
    })) {
        Ok(Ok(label)) => Ok(label),
        Ok(Err(error)) if error.is_uncertain() => {
            Err(OutputWorkerError::Uncertain(error.to_string()))
        }
        Ok(Err(error)) => Err(OutputWorkerError::Business(error.to_string())),
        Err(_) => Err(OutputWorkerError::Uncertain(
            "长截图贴图执行异常，创建结果无法确认".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pin::PinOrigin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_origin() -> PinOrigin {
        PinOrigin {
            x: -12.5,
            y: 8.25,
            width: 320.5,
            height: 640.75,
        }
    }

    fn best_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Best);
            let mut writer = encoder.write_header().expect("测试 PNG 头应可写入");
            writer
                .write_image_data(rgba)
                .expect("测试 PNG 像素应可写入");
        }
        encoded
    }

    #[test]
    fn registers_trusted_origin_after_clipboard_success() {
        let rgba = [
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
        ];
        let source_png = crate::screenshot::encode_png(&rgba, 2, 2).expect("测试 PNG");
        let clipboard_png = best_png(2, 2, &rgba);
        assert_ne!(source_png, clipboard_png, "夹具必须使用不同 PNG 编码");
        let artifact = LongshotOutputArtifact {
            png: Arc::new(source_png),
            origin: test_origin(),
        };
        let origins = PinOriginRegistry::default();
        let writes = AtomicUsize::new(0);

        copy_longshot_artifact(&artifact, &origins, |image| {
            writes.fetch_add(1, Ordering::SeqCst);
            assert_eq!((image.width, image.height), (2, 2));
            assert_eq!(image.bytes.as_ref(), rgba);
            Ok(())
        })
        .expect("剪贴板写入成功");

        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert_eq!(origins.lookup(&clipboard_png), Some(test_origin()));
    }

    #[test]
    fn clipboard_failure_never_registers_an_origin() {
        let rgba = [1, 2, 3, 255];
        let source_png = crate::screenshot::encode_png(&rgba, 1, 1).expect("测试 PNG");
        let artifact = LongshotOutputArtifact {
            png: Arc::new(source_png.clone()),
            origin: test_origin(),
        };
        let origins = PinOriginRegistry::default();

        let error = copy_longshot_artifact(&artifact, &origins, |_| {
            Err("clipboard unavailable".to_string())
        })
        .expect_err("剪贴板错误必须透传");

        assert_eq!(error, "clipboard unavailable");
        assert_eq!(origins.lookup(&source_png), None);
    }

    #[test]
    fn pin_forwards_the_same_png_arc_and_trusted_origin() {
        let png = Arc::new(vec![1, 2, 3, 4]);
        let artifact = LongshotOutputArtifact {
            png: Arc::clone(&png),
            origin: test_origin(),
        };

        let label = pin_longshot_artifact(&artifact, |received, origin| {
            assert!(Arc::ptr_eq(&received, &png));
            assert_eq!(origin, test_origin());
            Ok("pin-image-longshot".to_string())
        })
        .expect("Pin 应成功");

        assert_eq!(label, "pin-image-longshot");
    }

    #[test]
    fn pin_error_certainty_is_preserved_for_retry_policy() {
        let artifact = LongshotOutputArtifact {
            png: Arc::new(vec![1, 2, 3, 4]),
            origin: test_origin(),
        };
        let not_created = pin_longshot_artifact(&artifact, |_, _| {
            Err(ScreenshotPinCreateError::NotCreated {
                message: "not created".to_string(),
            })
        });
        assert!(matches!(
            not_created,
            Err(OutputWorkerError::Business(message)) if message == "not created"
        ));

        let uncertain = pin_longshot_artifact(&artifact, |_, _| {
            Err(ScreenshotPinCreateError::Uncertain {
                attempted_label: "pin-image-attempted".to_string(),
                message: "unknown outcome".to_string(),
            })
        });
        assert!(matches!(
            uncertain,
            Err(OutputWorkerError::Uncertain(message)) if message == "unknown outcome"
        ));

        let panic = pin_longshot_artifact(&artifact, |_, _| panic!("injected Pin panic"));
        assert!(matches!(
            panic,
            Err(OutputWorkerError::Uncertain(message)) if message.contains("结果无法确认")
        ));
    }
}
