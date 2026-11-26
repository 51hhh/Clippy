use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

pub const DETECTOR: &str = "pp-ocrv6_small_det.onnx";
pub const RECOGNIZER: &str = "pp-ocrv6_small_rec.onnx";
pub const DICTIONARY: &str = "ppocrv6_dict.txt";
pub const LINE_ORIENTATION: &str = "line-orientation.onnx";

#[cfg(target_os = "linux")]
pub const RUNTIME: &str = "libonnxruntime.so";
#[cfg(target_os = "macos")]
pub const RUNTIME: &str = "libonnxruntime.dylib";
#[cfg(windows)]
pub const RUNTIME: &str = "onnxruntime.dll";

pub fn verify_models(directory: &Path) -> Result<(), String> {
    let pins: [(&str, &str, &str); 4] =
        serde_json::from_str(include_str!("../models.json")).expect("模型清单随程序构建");
    for (name, _, expected) in pins {
        let actual = model_hash(&directory.join(name))
            .map_err(|error| format!("Cannot read OCR model {name}: {error}"))?;
        if actual != expected {
            return Err(format!("OCR model checksum mismatch: {name}"));
        }
    }
    Ok(())
}

fn model_hash(path: &Path) -> io::Result<String> {
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || !(1..=MAX_BYTES).contains(&metadata.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a non-empty model within the file budget",
        ));
    }
    let mut digest = Sha256::new();
    let bytes = io::copy(&mut File::open(path)?.take(MAX_BYTES + 1), &mut digest)?;
    if !(1..=MAX_BYTES).contains(&bytes) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "OCR model exceeds the file budget",
        ));
    }
    Ok(format!("{:x}", digest.finalize()))
}
