use crate::models::ContentType;
use crate::storage::StorageEngine;

fn editable_document(brightness: i32) -> crate::pin::commands::PinCanvasProject {
    crate::pin::commands::PinCanvasProject {
        renderer_version: crate::pin::render_v2::RENDERER_VERSION,
        source_width: 2,
        source_height: 1,
        annotations: serde_json::json!([]),
        adjustments: serde_json::json!({
            "grayscale": false,
            "brightness": brightness,
            "contrast": 0,
            "saturation": 0,
            "cornerRadius": 0
        }),
    }
}

#[test]
fn pin_renderer_and_shared_storage_round_trip_the_root_revision() {
    let storage = StorageEngine::new_in_memory().unwrap();
    let source =
        crate::screenshot::encode_png(&[10, 20, 30, 255, 200, 180, 160, 255], 2, 1).unwrap();
    let source_hash = crate::clipboard_watcher::content::compute_hash(&source);
    let root = storage
        .insert_clip(
            &ContentType::Image,
            None,
            None,
            Some(&source),
            &source_hash,
            source.len() as i64,
            false,
        )
        .unwrap();

    let document = editable_document(25);
    let rendered = crate::pin::output::render_document(&source, Some(&document)).unwrap();
    crate::pin::output::register_source_revision(
        &storage,
        &source,
        Some(root.id),
        &document,
        &rendered,
    )
    .unwrap();
    let rendered_hash = crate::clipboard_watcher::content::compute_hash(&rendered);
    let rendered_clip = storage
        .insert_clip(
            &ContentType::Image,
            None,
            None,
            Some(&rendered),
            &rendered_hash,
            rendered.len() as i64,
            false,
        )
        .unwrap();

    let restored = storage
        .get_image_revision_for_clip(rendered_clip.id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.source_png, source);
    assert_eq!(restored.adjustments["brightness"], 25);
}
