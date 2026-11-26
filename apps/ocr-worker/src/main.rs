use clippy_ocr_protocol::{
    Line, MAX_REPLY_BYTES, REQUEST_SIZE, Reply, Request, rgba_bytes, validate_lines,
};
use image::RgbImage;
use oar_ocr::prelude::*;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

const MODELS: [(&str, &str); 3] = [
    (
        "pp-ocrv6_small_det.onnx",
        "d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e",
    ),
    (
        "pp-ocrv6_small_rec.onnx",
        "5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634",
    ),
    (
        "ppocrv6_dict.txt",
        "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d",
    ),
];

#[cfg(target_os = "linux")]
const RUNTIME: &str = "libonnxruntime.so";
#[cfg(target_os = "macos")]
const RUNTIME: &str = "libonnxruntime.dylib";
#[cfg(windows)]
const RUNTIME: &str = "onnxruntime.dll";

fn model_hash(path: &Path) -> Result<String, Box<dyn Error>> {
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    let mut digest = Sha256::new();
    let bytes = io::copy(&mut File::open(path)?.take(MAX_BYTES + 1), &mut digest)?;
    if bytes == 0 || bytes > MAX_BYTES {
        return Err("OCR model exceeds the file budget".into());
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn open_engine(bundle: &Path) -> Result<OAROCR, Box<dyn Error>> {
    let models = bundle.join("models");
    for (name, expected) in MODELS {
        let actual = model_hash(&models.join(name))
            .map_err(|error| format!("Cannot read OCR model {name}: {error}"))?;
        if actual != expected {
            return Err(format!("OCR model checksum mismatch: {name}").into());
        }
    }
    let environment = ort::init_from(bundle.join(RUNTIME))?.build()?;
    oar_ocr::core::inference::set_ort_environment(environment)?;
    // 预设查找本地固定资源，工作目录仅用于这个独立进程。
    std::env::set_current_dir(models)?;
    let session = oar_ocr::core::config::onnx::OrtSessionConfig::default()
        .with_intra_threads(2)
        .with_inter_threads(1)
        .with_parallel_execution(false)
        .add_config_entry("session.intra_op.allow_spinning", "0")
        .add_config_entry("session.inter_op.allow_spinning", "0");
    Ok(OAROCRBuilder::pp_ocrv6(PpOcrV6Size::Small)
        .ort_session(session)
        .build()?)
}

fn composite_rgba(mut pixels: Vec<u8>) -> Vec<u8> {
    // 与旧模型使用同一个白底合成规则，原地压成 RGB 以减少一份全图分配。
    for index in 0..pixels.len() / 4 {
        let source = index * 4;
        let [red, green, blue, alpha]: [u8; 4] = pixels[source..source + 4].try_into().unwrap();
        let alpha = u16::from(alpha);
        let destination = index * 3;
        for (offset, channel) in [red, green, blue].into_iter().enumerate() {
            pixels[destination + offset] =
                ((u16::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    pixels.truncate(pixels.len() / 4 * 3);
    pixels
}

fn recognize(
    engine: &OAROCR,
    request: Request,
    rgba: Vec<u8>,
) -> Result<Vec<Line>, Box<dyn Error>> {
    let image = RgbImage::from_raw(request.width, request.height, composite_rgba(rgba))
        .expect("已按请求尺寸读取完整像素");
    let [result]: [_; 1] = engine
        .predict(vec![image])?
        .try_into()
        .expect("单图请求只返回一项识别结果");
    let mut lines = Vec::new();
    for region in result.text_regions {
        if let Some((text, confidence)) = region.text_with_confidence() {
            if text.is_empty() {
                continue;
            }
            let quad: [_; 4] = region
                .bounding_box
                .points
                .as_slice()
                .try_into()
                .map_err(|_| "OCR region does not have four source points")?;
            lines.push(Line {
                text: text.into(),
                confidence: f64::from(confidence),
                quad: quad.map(|point| [f64::from(point.x), f64::from(point.y)]),
            });
        }
    }
    validate_lines(&lines, request)?;
    Ok(lines)
}

fn write_reply(output: &mut impl Write, reply: Reply) -> Result<(), Box<dyn Error>> {
    let mut bytes = serde_json::to_vec(&reply)?;
    if bytes.len() > MAX_REPLY_BYTES {
        bytes = serde_json::to_vec(&Reply {
            id: reply.id,
            result: Err("OCR output exceeds the pipe budget".into()),
        })?;
    }
    output.write_all(&(bytes.len() as u32).to_le_bytes())?;
    output.write_all(&bytes)?;
    output.flush()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let bundle = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("Expected an absolute OCR bundle directory")?;
    let engine = open_engine(&bundle).map_err(|error| error.to_string());
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut header = [0; REQUEST_SIZE];
    while input.read(&mut header[..1])? != 0 {
        input.read_exact(&mut header[1..])?;
        let request = Request::decode(header)?;
        let mut pixels = vec![0; rgba_bytes(request.width, request.height)?];
        input.read_exact(&mut pixels)?;
        let result = match &engine {
            Ok(engine) => recognize(engine, request, pixels).map_err(|error| error.to_string()),
            Err(error) => Err(error.clone()),
        };
        write_reply(
            &mut output,
            Reply {
                id: request.id,
                result,
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::composite_rgba;

    #[test]
    fn composites_in_place_with_legacy_white_background() {
        assert_eq!(
            composite_rgba(vec![10, 20, 30, 255, 255, 0, 64, 128, 0, 0, 0, 0]),
            vec![10, 20, 30, 255, 127, 159, 255, 255, 255]
        );
    }
}
