use crate::{project::ProjectDocument, render_v2};
use image::RgbaImage;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Marker,
    Rectangle,
    Ellipse,
    Highlight,
    Line,
    Arrow,
    Measure,
    Text,
    Blur,
    Mosaic,
    Spotlight,
    Magnifier,
}

impl Tool {
    pub const ALL: [Self; 13] = [
        Self::Pen,
        Self::Marker,
        Self::Rectangle,
        Self::Ellipse,
        Self::Highlight,
        Self::Line,
        Self::Arrow,
        Self::Measure,
        Self::Text,
        Self::Blur,
        Self::Mosaic,
        Self::Spotlight,
        Self::Magnifier,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pen => "Pen",
            Self::Marker => "Marker",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Highlight => "Highlight",
            Self::Line => "Line",
            Self::Arrow => "Arrow",
            Self::Measure => "Measure",
            Self::Text => "Text",
            Self::Blur => "Blur",
            Self::Mosaic => "Mosaic",
            Self::Spotlight => "Spotlight",
            Self::Magnifier => "Magnifier",
        }
    }

    fn document_name(self) -> &'static str {
        match self {
            Self::Rectangle => "rect",
            Self::Ellipse => "ellipse",
            Self::Pen => "pen",
            Self::Marker => "marker",
            Self::Highlight => "highlight",
            Self::Line => "line",
            Self::Arrow => "arrow",
            Self::Measure => "measure",
            Self::Text => "text",
            Self::Blur => "blur",
            Self::Mosaic => "mosaic",
            Self::Spotlight => "spotlight",
            Self::Magnifier => "magnifier",
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Adjustments {
    pub grayscale: bool,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub corner_radius: f32,
}

impl Adjustments {
    fn document(self) -> Value {
        json!({
            "grayscale": self.grayscale, "brightness": self.brightness, "contrast": self.contrast,
            "saturation": self.saturation, "cornerRadius": self.corner_radius,
        })
    }
}

#[derive(Clone)]
pub struct ImageProject {
    pub source: Arc<RgbaImage>,
    pub document: ProjectDocument,
}

impl ImageProject {
    pub fn render(&self) -> Result<RgbaImage, String> {
        render_v2::render_pixels(
            &self.source,
            (0, 0, self.source.width(), self.source.height()),
            &self.document.annotations,
            &self.document.adjustments,
        )
    }
}

pub struct Editor {
    source: Arc<RgbaImage>,
    annotations: Vec<Value>,
    adjustments: Value,
    undone: Vec<Value>,
    next_id: u64,
}

impl Editor {
    pub fn new(source: Arc<RgbaImage>) -> Self {
        Self {
            source,
            annotations: Vec::new(),
            adjustments: Adjustments::default().document(),
            undone: Vec::new(),
            next_id: 0,
        }
    }

    pub fn from_project(project: ImageProject) -> Result<Self, String> {
        crate::project::validate_canvas_document(
            &project.document.annotations,
            &project.document.adjustments,
            project.source.width(),
            project.source.height(),
        )?;
        Ok(Self {
            source: project.source,
            annotations: serde_json::from_value(project.document.annotations)
                .expect("已经校验过工程标注数组"),
            adjustments: project.document.adjustments,
            undone: Vec::new(),
            next_id: 0,
        })
    }

    pub fn snapshot(&self) -> ImageProject {
        ImageProject {
            source: self.source.clone(),
            document: ProjectDocument {
                annotations: Value::Array(self.annotations.clone()),
                adjustments: self.adjustments.clone(),
            },
        }
    }

    pub fn has_annotations(&self) -> bool {
        !self.annotations.is_empty()
    }

    pub fn has_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        if let Some(annotation) = self.annotations.pop() {
            self.undone.push(annotation);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(annotation) = self.undone.pop() {
            self.annotations.push(annotation);
            true
        } else {
            false
        }
    }

    pub fn adjust(&mut self, adjustments: Adjustments) {
        self.adjustments = adjustments.document();
    }

    pub fn change_grayscale_and_corners(&mut self, grayscale: bool, corners: f32) {
        self.adjustments["grayscale"] = json!(grayscale);
        self.adjustments["cornerRadius"] = json!(corners);
    }

    pub fn grayscale_and_corners(&self) -> (bool, f32) {
        (
            self.adjustments["grayscale"]
                .as_bool()
                .expect("工程灰度属性已校验"),
            self.adjustments["cornerRadius"]
                .as_f64()
                .expect("工程圆角属性已校验") as f32,
        )
    }

    pub fn annotate(
        &mut self,
        tool: Tool,
        points: &[[f32; 2]],
        color: &str,
        size: f32,
        text: &str,
    ) -> Result<bool, String> {
        let mut points = points.iter().map(|[x, y]| {
            [
                f64::from(x.clamp(0.0, 1.0)) * f64::from(self.source.width()),
                f64::from(y.clamp(0.0, 1.0)) * f64::from(self.source.height()),
            ]
        });
        let Some(first) = points.next() else {
            return Ok(false);
        };
        let noninitial = points.collect::<Vec<_>>();
        let last = *noninitial.last().unwrap_or(&first);
        if first == last && !matches!(tool, Tool::Text | Tool::Pen | Tool::Marker) {
            return Ok(false);
        }
        let id = loop {
            self.next_id += 1;
            let id = format!("slint-{}", self.next_id);
            if !self.annotations.iter().any(|value| value["id"] == id) {
                break id;
            }
        };
        let point = |[x, y]: [f64; 2]| json!({ "x": x, "y": y });
        let rect = json!({
            "x": first[0].min(last[0]), "y": first[1].min(last[1]),
            "width": (last[0] - first[0]).abs(), "height": (last[1] - first[1]).abs(),
        });
        let mut annotation = json!({ "id": id, "type": tool.document_name() });
        match tool {
            Tool::Blur | Tool::Mosaic | Tool::Spotlight | Tool::Magnifier => {
                annotation["rect"] = rect;
                annotation["effect"] = json!({
                    "blurRadius": 12, "mosaicCell": render_v2::DEFAULT_MOSAIC_CELL,
                    "spotlightDim": 0.65, "magnifierZoom": 2,
                });
            }
            _ => {
                annotation["color"] = json!(color);
                annotation["size"] = json!(size);
                match tool {
                    Tool::Pen | Tool::Marker => {
                        annotation["points"] = Value::Array(
                            std::iter::once(first)
                                .chain(noninitial)
                                .map(point)
                                .collect(),
                        );
                    }
                    Tool::Line | Tool::Arrow | Tool::Measure => {
                        annotation["from"] = point(first);
                        annotation["to"] = point(last);
                    }
                    Tool::Text => {
                        if text.trim().is_empty() {
                            return Err("Enter text before placing it on the image".to_string());
                        }
                        annotation["at"] = point(first);
                        annotation["text"] = json!(text);
                        annotation["fontFamily"] = json!("system-ui");
                    }
                    _ => annotation["rect"] = rect,
                }
            }
        }
        self.annotations.push(annotation);
        if self.annotations.len() > 10_000 {
            self.annotations.pop();
            return Err("Too many annotations".to_string());
        }
        self.undone.clear();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn editable_document_survives_render_undo_redo_and_reopen() {
        let image = Arc::new(RgbaImage::from_pixel(48, 32, Rgba([24, 77, 99, 255])));
        let mut editor = Editor::new(image.clone());
        assert!(editor
            .annotate(
                Tool::Rectangle,
                &[[0.2, 0.2], [0.8, 0.8]],
                "#ff3b30",
                3.0,
                ""
            )
            .unwrap());
        let rendered = editor.snapshot().render().unwrap();
        assert_ne!(rendered, *image);
        assert!(editor.undo());
        assert_eq!(editor.snapshot().render().unwrap(), *image);
        assert!(editor.redo());
        let reopened = Editor::from_project(editor.snapshot()).unwrap();
        assert_eq!(reopened.snapshot().render().unwrap(), rendered);
    }
}
