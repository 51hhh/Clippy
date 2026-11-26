use serde::{Deserialize, Serialize};

pub const WORKER_NAME: &str = if cfg!(windows) {
    "clippy-ocr-worker.exe"
} else {
    "clippy-ocr-worker"
};
pub const ENGINE_ID: &str = "ppocrv6-small-rust-v2";
pub const REQUEST_SIZE: usize = 20;
pub const MAX_REPLY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_LINES: usize = 512;
pub const MAX_LINE_BYTES: usize = 64 * 1024;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAX_IMAGE_EDGE: u32 = 16_384;

pub fn rgba_bytes(width: u32, height: u32) -> Result<usize, &'static str> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_PIXELS || width > MAX_IMAGE_EDGE || height > MAX_IMAGE_EDGE {
        return Err("OCR image exceeds the pixel budget");
    }
    Ok(pixels as usize * 4)
}

#[derive(Clone, Copy)]
pub struct Request {
    pub id: u64,
    pub width: u32,
    pub height: u32,
}

impl Request {
    pub fn encode(self) -> [u8; REQUEST_SIZE] {
        let mut bytes = [0; REQUEST_SIZE];
        bytes[..4].copy_from_slice(b"OCR1");
        bytes[4..12].copy_from_slice(&self.id.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.width.to_le_bytes());
        bytes[16..].copy_from_slice(&self.height.to_le_bytes());
        bytes
    }

    pub fn decode(bytes: [u8; REQUEST_SIZE]) -> Result<Self, &'static str> {
        if &bytes[..4] != b"OCR1" {
            return Err("Invalid OCR request header");
        }
        Ok(Self {
            id: u64::from_le_bytes(bytes[4..12].try_into().unwrap()),
            width: u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            height: u32::from_le_bytes(bytes[16..].try_into().unwrap()),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub id: u64,
    pub result: Result<Vec<Line>, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub text: String,
    pub quad: [[f64; 2]; 4],
    pub confidence: f64,
}

pub fn validate_lines(lines: &[Line], request: Request) -> Result<(), &'static str> {
    let mut text_bytes = 0;
    if lines.len() > MAX_LINES {
        return Err("OCR reply exceeds the line limit");
    }
    for line in lines {
        text_bytes += line.text.len();
        if line.text.is_empty() || line.text.len() > MAX_LINE_BYTES || text_bytes > MAX_TEXT_BYTES {
            return Err("OCR reply violates the text budget");
        }
        if !(0.0..=1.0).contains(&line.confidence)
            || line.quad.iter().any(|[x, y]| {
                !(0.0..=f64::from(request.width)).contains(x)
                    || !(0.0..=f64::from(request.height)).contains(y)
            })
        {
            return Err("OCR reply contains invalid source geometry or confidence");
        }
    }
    Ok(())
}
