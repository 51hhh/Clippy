mod engine;

use clippy_ocr_protocol::{
    Line, MAX_REPLY_BYTES, REQUEST_SIZE, Reply, Request, rgba_bytes, validate_lines,
};
use engine::Engine;
use image::RgbImage;
use std::{
    error::Error,
    io::{self, Read, Write},
    path::PathBuf,
};

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
    engine: &Engine,
    request: Request,
    rgba: Vec<u8>,
) -> Result<Vec<Line>, Box<dyn Error>> {
    let image = RgbImage::from_raw(request.width, request.height, composite_rgba(rgba))
        .expect("已按请求尺寸读取完整像素");
    let lines = engine.recognize(image)?;
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
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("Expected an absolute OCR bundle directory")?;
    let engine = Engine::open(&bundle).map_err(|error| error.to_string());
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
