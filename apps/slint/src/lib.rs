slint::include_modules!();

use clippy_core::models::{ClipItem, ContentType};
use clippy_platform::identity::DeploymentIdentity;
use slint::SharedString;

mod app;

pub const UI_BACKEND: &str = "Slint";
pub const FRAMEWORK_VERSION: &str = "1.19.0";

pub fn run(identity: &'static DeploymentIdentity) -> anyhow::Result<()> {
    app::run(identity)
}

fn clip_row(clip: ClipItem) -> ClipRow {
    let kind = match &clip.content_type {
        ContentType::Text => "Text",
        ContentType::Html => "Rich text",
        ContentType::Image => "Image",
    };
    let preview = if clip.is_sensitive {
        "Sensitive content hidden".to_string()
    } else {
        clip.text_content
            .as_deref()
            .map(|text| truncate_preview(text, 160))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| match &clip.content_type {
                ContentType::Image => "Image content".to_string(),
                ContentType::Html => "Rich text content".to_string(),
                ContentType::Text => "Empty text".to_string(),
            })
    };
    ClipRow {
        id: clip.id.to_string().into(),
        kind: SharedString::from(kind),
        preview: preview.into(),
        metadata: format!("{} bytes", clip.byte_size).into(),
        favorite: clip.is_favorite,
        sensitive: clip.is_sensitive,
    }
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let preview = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

pub fn build_label(identity: &DeploymentIdentity) -> String {
    format!(
        "v{} · {UI_BACKEND} {FRAMEWORK_VERSION} · {}",
        env!("CARGO_PKG_VERSION"),
        identity.channel.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clippy_platform::identity::{PRODUCTION_IDENTITY, SLINT_QA_IDENTITY};

    #[test]
    fn framework_identity_is_explicit() {
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.2.0");
        assert_eq!(UI_BACKEND, "Slint");
        assert_eq!(FRAMEWORK_VERSION, "1.19.0");
        assert_eq!(clippy_core::PRODUCT_NAME, "Clippy");
    }

    #[test]
    fn build_label_exposes_the_selected_distribution_channel() {
        assert!(build_label(&PRODUCTION_IDENTITY).ends_with("production"));
        assert!(build_label(&SLINT_QA_IDENTITY).ends_with("qa"));
    }

    #[test]
    fn sensitive_rows_never_copy_their_text_into_the_ui_model() {
        let row = clip_row(ClipItem {
            id: 1,
            content_type: ContentType::Text,
            text_content: Some("secret-token".into()),
            html_content: None,
            image_data: None,
            content_hash: "hash".into(),
            is_favorite: false,
            is_sensitive: true,
            created_at: 0,
            byte_size: 12,
        });
        assert_eq!(row.preview.as_str(), "Sensitive content hidden");
        assert!(row.sensitive);
    }

    #[test]
    fn preview_truncation_respects_unicode_boundaries() {
        let preview = truncate_preview(&format!("{}尾", "界".repeat(160)), 160);
        assert_eq!(preview.chars().count(), 161);
        assert!(preview.ends_with('…'));
    }
}
