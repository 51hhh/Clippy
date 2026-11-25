//! Tesseract 命令协议：参数、输出预算和 UTF-8 文本转换。

use super::process::run_recognition_process;
use super::{OcrLine, OcrParagraph, OcrResult, StructuredOcr};
use std::collections::HashMap;
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

pub(super) async fn recognize_with_layout(
    executable: &Path,
    png_bytes: &[u8],
    timeout: Duration,
    dimensions: (u32, u32),
    reason: Option<String>,
) -> Result<StructuredOcr, String> {
    let mut command = tokio::process::Command::new(executable);
    command.args(["stdin", "stdout", "-l", "eng+chi_sim", "--psm", "11", "tsv"]);
    let stdout = run_recognition_process(
        command,
        png_bytes,
        timeout,
        OCR_STDOUT_LIMIT,
        OCR_STDERR_LIMIT,
    )
    .await?;
    parse_layout(&parse_output(stdout)?, dimensions, reason)
}

struct TextLine {
    text: String,
    scores: Vec<f64>,
    rect: [u32; 4],
    paragraph: u32,
}

fn parse_layout(
    tsv: &str,
    (width, height): (u32, u32),
    reason: Option<String>,
) -> Result<StructuredOcr, String> {
    let mut lines: Vec<TextLine> = Vec::new();
    let mut line_index = HashMap::new();
    let mut paragraph_index = HashMap::new();
    let mut paragraphs: Vec<OcrParagraph> = Vec::new();
    let mut total_text = 0;
    for row in tsv.lines().skip(1) {
        let values = row.splitn(12, '\t').collect::<Vec<_>>();
        if values.len() != 12 || values[0] != "5" || values[11].trim().is_empty() {
            continue;
        }
        let invalid = || "Tesseract returned an invalid text box".to_owned();
        let read = |column: usize| values[column].parse::<u32>().map_err(|_| invalid());
        let group = (read(1)?, read(2)?, read(3)?);
        let (x, y) = (read(6)?.min(width), read(7)?.min(height));
        let right = x.saturating_add(read(8)?).min(width);
        let bottom = y.saturating_add(read(9)?).min(height);
        if x == right || y == bottom {
            continue;
        }
        let confidence = values[10].parse::<f64>().map_err(|_| invalid())? / 100.0;
        if !confidence.is_finite() {
            return Err(invalid());
        }
        let word = values[11].trim();
        total_text += word.len() + 1;
        if total_text > 1024 * 1024 {
            return Err("OCR text exceeds its output budget".into());
        }
        let paragraph = *paragraph_index.entry(group).or_insert_with(|| {
            let id = paragraphs.len() as u32;
            paragraphs.push(OcrParagraph {
                id,
                line_ids: Vec::new(),
                reading_order: id,
            });
            id
        });
        let index = *line_index.entry((group, read(4)?)).or_insert_with(|| {
            let index = lines.len();
            lines.push(TextLine {
                text: String::new(),
                scores: Vec::new(),
                rect: [x, y, right, bottom],
                paragraph,
            });
            paragraphs[paragraph as usize].line_ids.push(index as u32);
            index
        });
        if lines.len() > 512 {
            return Err("OCR line count exceeds its output budget".into());
        }
        let line = &mut lines[index];
        let confidence = confidence.clamp(0.0, 1.0);
        if !line.text.is_empty() {
            line.text.push(' ');
            line.scores.push(confidence);
        }
        line.text.push_str(word);
        line.scores
            .extend(std::iter::repeat_n(confidence, word.chars().count()));
        line.rect = [
            line.rect[0].min(x),
            line.rect[1].min(y),
            line.rect[2].max(right),
            line.rect[3].max(bottom),
        ];
    }
    let lines = lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let [left, top, right, bottom] = line.rect.map(f64::from);
            OcrLine {
                id: index as u32,
                reading_order: index as u32,
                paragraph_id: line.paragraph,
                text: line.text,
                accepted: true,
                confidence: line.scores.iter().sum::<f64>() / line.scores.len() as f64,
                char_confidences: line.scores,
                quad: vec![[left, top], [right, top], [right, bottom], [left, bottom]],
            }
        })
        .collect::<Vec<_>>();
    let text = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut result = StructuredOcr::tesseract(width, height, text, reason);
    result.pipeline.id = "tesseract-tsv-v1".into();
    result.pipeline.layout_executed = !lines.is_empty();
    result.pipeline.layout_reason = lines.is_empty().then(|| "no_text".into());
    result.lines = lines;
    result.paragraphs = paragraphs;
    Ok(result)
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

    #[test]
    fn tsv_keeps_line_geometry_and_paragraph_order() {
        let input = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                     5\t1\t1\t1\t1\t1\t5\t6\t10\t7\t96.5\tHello\n\
                     5\t1\t1\t1\t1\t2\t19\t6\t15\t8\t93\t世界\n\
                     5\t1\t1\t2\t1\t1\t8\t30\t12\t7\t90\tNext";
        let result = parse_layout(input, (40, 50), None).unwrap();
        assert_eq!(result.text, "Hello 世界\nNext");
        assert_eq!(
            result.lines[0].quad,
            vec![[5., 6.], [34., 6.], [34., 14.], [5., 14.]]
        );
        assert_eq!(result.lines[0].char_confidences.len(), 8);
        assert_eq!(result.paragraphs[1].line_ids, [1]);
    }
}
