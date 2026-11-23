//! Tesseract 命令协议：参数、输出预算和 UTF-8 文本转换。

use super::process::run_recognition_process;
use super::OcrResult;
use std::path::Path;
use std::time::Duration;

pub(super) const RECOGNITION_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const OCR_STDOUT_LIMIT: usize = 4 * 1024 * 1024;
pub(super) const OCR_STDERR_LIMIT: usize = 64 * 1024;

pub(super) fn parse_output(stdout: Vec<u8>) -> OcrResult {
    String::from_utf8(stdout)
        .map(|text| text.trim().to_string())
        .map_err(|_| "OCR 输出不是有效 UTF-8".into())
}

pub(super) async fn recognize_with_timeout(
    executable: &Path,
    png_bytes: &[u8],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> OcrResult {
    let mut command = tokio::process::Command::new(executable);
    command.args(["stdin", "stdout", "-l", "eng+chi_sim"]);
    let stdout =
        run_recognition_process(command, png_bytes, timeout, stdout_limit, stderr_limit).await?;
    parse_output(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_parser_trims_valid_utf8_and_rejects_invalid_bytes() {
        assert_eq!(
            parse_output("  识别结果\n".as_bytes().to_vec()).unwrap(),
            "识别结果"
        );
        assert!(parse_output(vec![0xff]).unwrap_err().contains("UTF-8"));
    }
}
