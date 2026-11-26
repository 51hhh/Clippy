use clippy_ocr_protocol::Line;
use image::{RgbImage, imageops::rotate180_in_place};
use oar_ocr::{
    core::{
        config::OrtSessionConfig,
        traits::{
            OrtConfigurable,
            adapter::{AdapterBuilder, ModelAdapter},
            task::ImageTaskInput,
        },
    },
    domain::{
        adapters::{
            TextDetectionAdapter, TextDetectionAdapterBuilder, TextLineOrientationAdapter,
            TextLineOrientationAdapterBuilder, TextRecognitionAdapter,
            TextRecognitionAdapterBuilder,
        },
        tasks::TextDetectionConfig,
    },
    oarocr::{CtcDecoding, RecognitionPreprocessing},
    processors::{LimitType, quad_box_order},
    utils::get_rotate_crop_image,
};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{self, File},
    io::{self, Read},
    path::Path,
    sync::Arc,
};

const DETECTOR: &str = "pp-ocrv6_small_det.onnx";
const RECOGNIZER: &str = "pp-ocrv6_small_rec.onnx";
const DICTIONARY: &str = "ppocrv6_dict.txt";
const LINE_ORIENTATION: &str = "line-orientation.onnx";

#[cfg(target_os = "linux")]
const RUNTIME: &str = "libonnxruntime.so";
#[cfg(target_os = "macos")]
const RUNTIME: &str = "libonnxruntime.dylib";
#[cfg(windows)]
const RUNTIME: &str = "onnxruntime.dll";

pub(super) struct Engine {
    detector: TextDetectionAdapter,
    orientation: TextLineOrientationAdapter,
    recognizer: TextRecognitionAdapter,
}

impl Engine {
    pub(super) fn open(bundle: &Path) -> Result<Self, Box<dyn Error>> {
        let models = bundle.join("models");
        let pins: [(String, String, String); 4] =
            serde_json::from_str(include_str!("../models.json")).expect("模型清单随程序构建");
        for (name, _, expected) in pins {
            let actual = model_hash(&models.join(&name))
                .map_err(|error| format!("Cannot read OCR model {name}: {error}"))?;
            if actual != expected {
                return Err(format!("OCR model checksum mismatch: {name}").into());
            }
        }
        let environment = ort::init_from(bundle.join(RUNTIME))?.build()?;
        oar_ocr::core::inference::set_ort_environment(environment)?;
        let session = OrtSessionConfig::default()
            .with_intra_threads(2)
            .with_inter_threads(1)
            .with_parallel_execution(false)
            .add_config_entry("session.intra_op.allow_spinning", "0")
            .add_config_entry("session.inter_op.allow_spinning", "0");
        // 拆出分类阶段以沿用旧版的 0.8 翻转门限，检测保持原 PP-OCRv6 small 预设。
        let detector = TextDetectionAdapterBuilder::new()
            .with_ort_config(session.clone())
            .with_config(TextDetectionConfig {
                score_threshold: 0.2,
                box_threshold: 0.45,
                unclip_ratio: 1.4,
                max_candidates: 3000,
                limit_side_len: Some(960),
                limit_type: Some(LimitType::Max),
                max_side_len: Some(4000),
            })
            .build(models.join(DETECTOR))?;
        let orientation = TextLineOrientationAdapterBuilder::new()
            .with_ort_config(session.clone())
            .build(models.join(LINE_ORIENTATION))?;
        let recognizer = TextRecognitionAdapterBuilder::new()
            .with_ort_config(session)
            .character_dict(
                fs::read_to_string(models.join(DICTIONARY))?
                    .lines()
                    .map(str::to_owned)
                    .collect(),
            )
            .build(models.join(RECOGNIZER))?;
        Ok(Self {
            detector,
            orientation,
            recognizer,
        })
    }

    pub(super) fn recognize(&self, image: RgbImage) -> Result<Vec<Line>, Box<dyn Error>> {
        const CROP_BATCH: usize = 32;
        const RECOGNITION_BATCH: usize = 4;
        const FLIP_CONFIDENCE: f32 = 0.8;
        let image = Arc::new(image);
        let [detections]: [_; 1] = self
            .detector
            .execute(
                ImageTaskInput::from_arc_images(vec![Arc::clone(&image)]),
                None,
            )?
            .detections
            .try_into()
            .expect("单图请求只返回一项检测结果");
        let boxes: Vec<_> = detections
            .into_iter()
            .map(|detection| detection.bbox)
            .collect();
        let mut lines = Vec::new();
        // 长图按组持有裁切像素。几何校正只改变送往识别器的裁片。
        for indices in quad_box_order(&boxes).chunks(CROP_BATCH) {
            let mut crops = indices
                .iter()
                .map(|&index| get_rotate_crop_image(&image, &boxes[index].points).map(Arc::new))
                .collect::<Result<Vec<_>, _>>()?;
            let orientations = self
                .orientation
                .execute(ImageTaskInput::from_arc_images(crops.clone()), None)?;
            for (crop, classes) in crops.iter_mut().zip(orientations.classifications) {
                let class = classes.first().expect("行方向模型返回至少一项分类");
                if class.class_id == 1 && class.score >= FLIP_CONFIDENCE {
                    rotate180_in_place(Arc::get_mut(crop).expect("方向分类已释放裁片引用"));
                }
            }
            let readings = self.recognizer.read_lines(
                &crops,
                RecognitionPreprocessing::Standard,
                CtcDecoding::Standard,
                RECOGNITION_BATCH,
            )?;
            for (&index, reading) in indices.iter().zip(readings) {
                if reading.text.is_empty() {
                    continue;
                }
                let quad: [_; 4] = boxes[index]
                    .points
                    .as_slice()
                    .try_into()
                    .map_err(|_| "OCR region does not have four source points")?;
                lines.push(Line {
                    text: reading.text,
                    confidence: reading.confidence,
                    quad: quad.map(|point| [f64::from(point.x), f64::from(point.y)]),
                });
            }
        }
        Ok(lines)
    }
}

fn model_hash(path: &Path) -> Result<String, Box<dyn Error>> {
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    let mut digest = Sha256::new();
    let bytes = io::copy(&mut File::open(path)?.take(MAX_BYTES + 1), &mut digest)?;
    if bytes == 0 || bytes > MAX_BYTES {
        return Err("OCR model exceeds the file budget".into());
    }
    Ok(format!("{:x}", digest.finalize()))
}
