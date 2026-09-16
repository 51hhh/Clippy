//! OCR 子进程和查看器共用的有界结构化结果。
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrLine {
    pub id: u32,
    pub quad: Vec<[f64; 2]>,
    pub text: String,
    pub accepted: bool,
    pub confidence: f64,
    pub char_confidences: Vec<f64>,
    pub paragraph_id: u32,
    pub reading_order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrParagraph {
    pub id: u32,
    pub line_ids: Vec<u32>,
    pub reading_order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrPipeline {
    pub id: String,
    pub engine: String,
    pub feature_schema: Option<String>,
    pub layout_executed: bool,
    pub layout_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructuredOcr {
    pub width: u32,
    pub height: u32,
    pub text: String,
    pub lines: Vec<OcrLine>,
    pub paragraphs: Vec<OcrParagraph>,
    pub pipeline: OcrPipeline,
    pub fallback_reason: Option<String>,
}

impl StructuredOcr {
    pub(super) fn tesseract(width: u32, height: u32, text: String, reason: Option<String>) -> Self {
        Self {
            width,
            height,
            text,
            lines: Vec::new(),
            paragraphs: Vec::new(),
            pipeline: OcrPipeline {
                id: "tesseract-v1".into(),
                engine: "tesseract".into(),
                feature_schema: None,
                layout_executed: false,
                layout_reason: Some("unstructured_backend".into()),
            },
            fallback_reason: reason,
        }
    }

    pub(super) fn validate_enhanced(
        &self,
        width: u32,
        height: u32,
        pipeline: &str,
    ) -> Result<(), String> {
        let invalid = || "OCR 增强结果不符合接口约定".to_string();
        if self.width != width
            || self.height != height
            || self.pipeline.id != pipeline
            || self.pipeline.engine != "ppocrv6+edgegnn"
            || self.pipeline.feature_schema.as_deref() != Some("clippy-edge-features-v1")
            || self.fallback_reason.is_some()
            || self.lines.len() > 512
            || self.paragraphs.len() > self.lines.len()
            || self.text.len() > 1024 * 1024
        {
            return Err(invalid());
        }
        let expected_reason = match self.lines.len() {
            0 => Some("no_text"),
            1 => Some("single_line"),
            _ => None,
        };
        if self.pipeline.layout_executed != (self.lines.len() > 1)
            || self.pipeline.layout_reason.as_deref() != expected_reason
        {
            return Err(invalid());
        }
        let mut lines = HashMap::new();
        let mut order = HashSet::new();
        let mut total_bytes = 0usize;
        for line in &self.lines {
            total_bytes = total_bytes.saturating_add(line.text.len());
            if lines.insert(line.id, line).is_some()
                || !order.insert(line.reading_order)
                || line.reading_order as usize >= self.lines.len()
                || line.quad.len() != 4
                || line.quad.iter().any(|point| {
                    !point[0].is_finite()
                        || !point[1].is_finite()
                        || point[0] < 0.0
                        || point[0] > f64::from(width)
                        || point[1] < 0.0
                        || point[1] > f64::from(height)
                })
                || !valid_probability(line.confidence)
                || line.char_confidences.len() != line.text.chars().count()
                || line
                    .char_confidences
                    .iter()
                    .any(|value| !valid_probability(*value))
                || line.text.len() > 64 * 1024
                || total_bytes > 1024 * 1024
            {
                return Err(invalid());
            }
        }
        let mut paragraphs: Vec<_> = self.paragraphs.iter().collect();
        paragraphs.sort_by_key(|paragraph| paragraph.reading_order);
        let mut paragraph_ids = HashSet::new();
        let mut members = HashSet::new();
        let mut texts = Vec::new();
        let mut next_order = 0;
        for (index, paragraph) in paragraphs.iter().enumerate() {
            if paragraph.reading_order as usize != index
                || !paragraph_ids.insert(paragraph.id)
                || paragraph.line_ids.is_empty()
            {
                return Err(invalid());
            }
            let mut text = Vec::new();
            for id in &paragraph.line_ids {
                let line = lines.get(id).ok_or_else(invalid)?;
                if !members.insert(*id)
                    || line.paragraph_id != paragraph.id
                    || line.reading_order != next_order
                {
                    return Err(invalid());
                }
                next_order += 1;
                if line.accepted && !line.text.is_empty() {
                    text.push(line.text.as_str());
                }
            }
            if !text.is_empty() {
                texts.push(text.join("\n"));
            }
        }
        if members.len() != lines.len() || self.text != texts.join("\n\n") {
            return Err(invalid());
        }
        Ok(())
    }
}

fn valid_probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> StructuredOcr {
        StructuredOcr {
            width: 100,
            height: 80,
            text: "A".into(),
            lines: vec![OcrLine {
                id: 4,
                quad: vec![[0., 0.], [20., 0.], [20., 10.], [0., 10.]],
                text: "A".into(),
                accepted: true,
                confidence: 0.9,
                char_confidences: vec![0.9],
                paragraph_id: 3,
                reading_order: 0,
            }],
            paragraphs: vec![OcrParagraph {
                id: 3,
                line_ids: vec![4],
                reading_order: 0,
            }],
            pipeline: OcrPipeline {
                id: "fixture".into(),
                engine: "ppocrv6+edgegnn".into(),
                feature_schema: Some("clippy-edge-features-v1".into()),
                layout_executed: false,
                layout_reason: Some("single_line".into()),
            },
            fallback_reason: None,
        }
    }

    #[test]
    fn preserves_unaccepted_raw_characters_without_copying_them() {
        let mut value = result();
        assert!(value.validate_enhanced(100, 80, "fixture").is_ok());
        value.lines[0].accepted = false;
        assert!(value.validate_enhanced(100, 80, "fixture").is_err());
        value.text.clear();
        assert!(value.validate_enhanced(100, 80, "fixture").is_ok());
        assert_eq!(value.lines[0].text, "A");
        assert_eq!(value.lines[0].char_confidences, vec![0.9]);
    }

    #[test]
    fn rejects_nonfinite_out_of_bounds_duplicate_and_missing_members() {
        let mut value = result();
        value.lines[0].quad[0][0] = f64::NAN;
        assert!(value.validate_enhanced(100, 80, "fixture").is_err());
        let mut value = result();
        value.lines[0].quad[0][0] = 101.;
        assert!(value.validate_enhanced(100, 80, "fixture").is_err());
        let mut value = result();
        value.paragraphs[0].line_ids = vec![4, 4];
        assert!(value.validate_enhanced(100, 80, "fixture").is_err());
        let mut value = result();
        value.paragraphs[0].line_ids = vec![9];
        assert!(value.validate_enhanced(100, 80, "fixture").is_err());
        let mut value = result();
        value.lines[0].char_confidences.clear();
        assert!(value.validate_enhanced(100, 80, "fixture").is_err());
    }

    #[test]
    fn rejects_unknown_fields_and_false_layout_provenance() {
        let mut json = serde_json::to_value(result()).unwrap();
        json["lines"][0]["surprise"] = true.into();
        assert!(serde_json::from_value::<StructuredOcr>(json).is_err());
        let mut value = result();
        value.pipeline.layout_executed = true;
        assert!(value.validate_enhanced(100, 80, "fixture").is_err());
        assert!(result()
            .validate_enhanced(100, 80, "different-model")
            .is_err());
    }
}
