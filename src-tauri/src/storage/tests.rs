use super::url_cache::{MAX_URL_META_ENTRIES, URL_META_TTL_SECS};
use super::*;
use crate::models::UrlMeta;
use std::time::Duration;

/// 构造一条文本类型的测试 ClipItem 插入参数。
fn insert_text(engine: &StorageEngine, text: &str, hash: &str) -> ClipItem {
    engine
        .insert_clip(
            &ContentType::Text,
            Some(text),
            None,
            None,
            hash,
            text.len() as i64,
            false,
        )
        .expect("插入失败")
}

fn insert_image(engine: &StorageEngine, hash: &str) -> ClipItem {
    engine
        .insert_clip(
            &ContentType::Image,
            None,
            None,
            Some(&[137, 80, 78, 71]),
            hash,
            4,
            false,
        )
        .expect("插入图片失败")
}

#[test]
fn bounded_code_scan_image_query_keeps_oversized_blob_out_of_rust_memory() {
    let engine = StorageEngine::new_in_memory().unwrap();
    // byte_size 刻意伪造为 1；识别读取必须只信任 SQLite BLOB 的真实 length()。
    engine
        .conn
        .execute(
            "INSERT INTO clips
                (content_type, text_content, html_content, image_data, content_hash,
                 is_favorite, created_at, byte_size, is_sensitive)
             VALUES ('image', NULL, NULL, zeroblob(128), 'bounded-image-blob', 0, 0, 1, 0)",
            [],
        )
        .unwrap();
    let id = engine.conn.last_insert_rowid();

    // SQL 的 CASE 分支返回 NULL，而不是把 zeroblob(128) materialize 成 Vec<u8>。
    assert_eq!(
        engine.get_bounded_image_for_code_scan(id, 16).unwrap(),
        BoundedImageData::TooLarge
    );
    assert_eq!(
        engine.get_bounded_image_for_code_scan(id, 128).unwrap(),
        BoundedImageData::Bytes(vec![0; 128])
    );
}

#[test]
fn test_insert_and_query() {
    let engine = StorageEngine::new_in_memory().unwrap();
    insert_text(&engine, "hello world", "hash_hw");

    let clips = engine.get_clips(None, false, 0, 10).unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].text_content.as_deref(), Some("hello world"));
    assert_eq!(clips[0].content_hash, "hash_hw");
    assert!(!clips[0].is_favorite);
}

#[test]
fn test_dedup_updates_timestamp() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clip1 = insert_text(&engine, "same content", "hash_same");
    let ts1 = clip1.created_at;
    std::thread::sleep(Duration::from_secs(1));
    let clip2 = insert_text(&engine, "same content", "hash_same");
    let ts2 = clip2.created_at;

    let clips = engine.get_clips(None, false, 0, 10).unwrap();
    assert_eq!(clips.len(), 1, "重复内容不应产生多条记录");
    assert!(ts2 > ts1, "重复插入应更新 created_at");
}

#[test]
fn test_search_matches_full_words_and_prefixes() {
    let engine = StorageEngine::new_in_memory().unwrap();
    insert_text(&engine, "apple pie recipe", "hash_apple");
    insert_text(&engine, "banana smoothie drink", "hash_banana");

    let results = engine.get_clips(Some("apple"), false, 0, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content_hash, "hash_apple");

    let prefix_results = engine.get_clips(Some("app"), false, 0, 10).unwrap();
    assert_eq!(prefix_results.len(), 1);
    assert_eq!(prefix_results[0].content_hash, "hash_apple");
}

#[test]
fn test_search_short_input_matches_substrings() {
    let engine = StorageEngine::new_in_memory().unwrap();
    insert_text(&engine, "apple pie recipe", "hash_apple");
    insert_text(&engine, "happy path", "hash_happy");

    let results = engine.get_clips(Some("p"), false, 0, 10).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|clip| clip.content_hash == "hash_apple"));
    assert!(results.iter().any(|clip| clip.content_hash == "hash_happy"));
}

#[test]
fn test_search_matches_chinese_text() {
    let engine = StorageEngine::new_in_memory().unwrap();
    insert_text(&engine, "这是一个剪贴板历史", "hash_cn");
    insert_text(&engine, "plain english", "hash_en");

    let results = engine.get_clips(Some("剪"), false, 0, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content_hash, "hash_cn");
}

#[test]
fn test_search_matches_ocr_text() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clip = insert_image(&engine, "hash_image");
    engine.set_ocr_text(clip.id, "Invoice Total 42").unwrap();

    let results = engine.get_clips(Some("Invoice"), false, 0, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content_hash, "hash_image");
}

#[test]
fn test_search_special_characters_are_literal() {
    let engine = StorageEngine::new_in_memory().unwrap();
    insert_text(&engine, "discount is 100%", "hash_percent");
    insert_text(&engine, "discount is 1000", "hash_plain");
    insert_text(&engine, "file_name", "hash_underscore");
    insert_text(&engine, "file-name", "hash_dash");

    let percent_results = engine.get_clips(Some("100%"), false, 0, 10).unwrap();
    assert_eq!(percent_results.len(), 1);
    assert_eq!(percent_results[0].content_hash, "hash_percent");

    let underscore_results = engine.get_clips(Some("file_"), false, 0, 10).unwrap();
    assert_eq!(underscore_results.len(), 1);
    assert_eq!(underscore_results[0].content_hash, "hash_underscore");
}

#[test]
fn test_search_respects_favorites_filter() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let favorite = insert_text(&engine, "apple favorite", "hash_fav");
    insert_text(&engine, "apple normal", "hash_normal");
    engine.toggle_favorite(favorite.id).unwrap();

    let results = engine.get_clips(Some("apple"), true, 0, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content_hash, "hash_fav");
    assert!(results[0].is_favorite);
}

#[test]
fn test_rebuild_fts_once_repairs_missing_index() {
    let engine = StorageEngine::new_in_memory().unwrap();
    insert_text(&engine, "apple pie recipe", "hash_apple");
    engine.conn.execute("DELETE FROM clips_fts", []).unwrap();

    let before: i64 = engine
        .conn
        .query_row(
            "SELECT COUNT(*) FROM clips_fts WHERE clips_fts MATCH 'apple'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, 0);

    engine.rebuild_fts_once("test_rebuild").unwrap();

    let after: i64 = engine
        .conn
        .query_row(
            "SELECT COUNT(*) FROM clips_fts WHERE clips_fts MATCH 'apple'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, 1);
}

#[test]
fn test_touch_clip_updates_timestamp() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clip = insert_text(&engine, "touch test", "hash_touch");
    let ts1 = clip.created_at;
    std::thread::sleep(Duration::from_secs(1));

    let updated = engine.touch_clip(clip.id).unwrap();
    assert!(updated.created_at > ts1, "touch_clip 应更新 created_at");
    assert_eq!(updated.id, clip.id);
    assert_eq!(updated.content_hash, "hash_touch");
}

#[test]
fn test_delete_clip() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clip = insert_text(&engine, "to be deleted", "hash_del");
    engine.delete_clip(clip.id).unwrap();

    let clips = engine.get_clips(None, false, 0, 10).unwrap();
    assert!(clips.is_empty(), "删除后列表应为空");
}

#[test]
fn test_toggle_favorite() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clip = insert_text(&engine, "toggle me", "hash_toggle");
    assert!(!clip.is_favorite);

    let new_state = engine.toggle_favorite(clip.id).unwrap();
    assert!(new_state, "第一次 toggle 应变为 true");
    let new_state2 = engine.toggle_favorite(clip.id).unwrap();
    assert!(!new_state2, "第二次 toggle 应变为 false");
}

#[test]
fn test_cleanup_preserves_favorites() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clips: Vec<ClipItem> = (1..=5)
        .map(|i| {
            if i > 1 {
                std::thread::sleep(Duration::from_millis(10));
            }
            insert_text(&engine, &format!("item {}", i), &format!("hash_{}", i))
        })
        .collect();
    engine.toggle_favorite(clips[2].id).unwrap();

    let removed = engine.cleanup_old_entries(2).unwrap();
    assert_eq!(removed.len(), 2, "应删除 2 条最旧的非收藏");
    let remaining = engine.get_clips(None, false, 0, 10).unwrap();
    assert_eq!(remaining.len(), 3, "应剩余 3 条（1 收藏 + 2 非收藏）");
    let fav_clip = engine.get_clip_by_id(clips[2].id).unwrap();
    assert!(fav_clip.is_favorite, "收藏条目不应被 cleanup 删除");
}

#[test]
fn test_clear_history_preserves_favorites() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let c1 = insert_text(&engine, "普通条目 1", "hash_c1");
    let c2 = insert_text(&engine, "普通条目 2", "hash_c2");
    let c3 = insert_text(&engine, "收藏条目", "hash_c3");
    engine.toggle_favorite(c3.id).unwrap();

    engine.clear_history().unwrap();

    let remaining = engine.get_clips(None, false, 0, 10).unwrap();
    assert_eq!(remaining.len(), 1, "clear_history 后只剩收藏");
    assert_eq!(remaining[0].id, c3.id, "剩余条目应为收藏的那条");
    assert!(engine.get_clip_by_id(c1.id).is_err());
    assert!(engine.get_clip_by_id(c2.id).is_err());
}

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
fn image_revisions_share_one_root_and_survive_history_cleanup() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let source =
        crate::screenshot::encode_png(&[10, 20, 30, 255, 200, 180, 160, 255], 2, 1).unwrap();
    let source_hash = crate::clipboard_watcher::content::compute_hash(&source);
    let root = engine
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

    let first_document = editable_document(10);
    let first_render = crate::pin::output::render_document(&source, Some(&first_document)).unwrap();
    let first_revision = crate::pin::output::register_source_revision(
        &engine,
        &source,
        Some(root.id),
        &first_document,
        &first_render,
    )
    .unwrap();
    let (blob_was_moved, asset_id): (bool, i64) = engine
        .conn
        .query_row(
            "SELECT image_data IS NULL, image_asset_id FROM clips WHERE id = ?1",
            params![root.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(blob_was_moved);
    assert!(asset_id > 0);
    assert_eq!(
        engine.get_clip_image(root.id).unwrap(),
        Some(source.clone())
    );
    assert_eq!(
        engine
            .get_bounded_image_for_code_scan(root.id, source.len())
            .unwrap(),
        BoundedImageData::Bytes(source.clone())
    );
    assert!(matches!(
        engine
            .get_bounded_image_snapshot(root.id, source.len())
            .unwrap()
            .unwrap()
            .image,
        BoundedImageData::Bytes(bytes) if bytes == source
    ));

    let first_hash = crate::clipboard_watcher::content::compute_hash(&first_render);
    let first_clip = engine
        .insert_clip(
            &ContentType::Image,
            None,
            None,
            Some(&first_render),
            &first_hash,
            first_render.len() as i64,
            false,
        )
        .unwrap();
    let restored = engine
        .get_image_revision_for_clip(first_clip.id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.source_png, source);
    assert_eq!(restored.adjustments["brightness"], 10);

    let second_document = editable_document(-20);
    let second_render =
        crate::pin::output::render_document(&restored.source_png, Some(&second_document)).unwrap();
    let incorrectly_resampled =
        crate::pin::output::render_document(&first_render, Some(&second_document)).unwrap();
    assert_ne!(
        second_render, incorrectly_resampled,
        "修订 3 必须从根图重放累计文档，不能从 PNG 2 二次采样"
    );
    let second_revision = crate::pin::output::register_source_revision(
        &engine,
        &restored.source_png,
        None,
        &second_document,
        &second_render,
    )
    .unwrap();
    assert_ne!(first_revision, second_revision);
    let second_hash = crate::clipboard_watcher::content::compute_hash(&second_render);
    let second_clip = engine
        .insert_clip(
            &ContentType::Image,
            None,
            None,
            Some(&second_render),
            &second_hash,
            second_render.len() as i64,
            false,
        )
        .unwrap();
    assert_eq!(
        engine
            .get_image_revision_for_clip(first_clip.id)
            .unwrap()
            .unwrap()
            .adjustments["brightness"],
        10
    );
    assert_eq!(
        engine
            .get_image_revision_for_clip(second_clip.id)
            .unwrap()
            .unwrap()
            .adjustments["brightness"],
        -20
    );
    assert_eq!(
        engine
            .conn
            .query_row("SELECT COUNT(*) FROM image_assets", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );

    let changed_pixels =
        crate::screenshot::encode_png(&[9, 20, 30, 255, 200, 180, 160, 255], 2, 1).unwrap();
    let changed_hash = crate::clipboard_watcher::content::compute_hash(&changed_pixels);
    let changed_clip = engine
        .insert_clip(
            &ContentType::Image,
            None,
            None,
            Some(&changed_pixels),
            &changed_hash,
            changed_pixels.len() as i64,
            false,
        )
        .unwrap();
    assert!(engine
        .get_image_revision_for_clip(changed_clip.id)
        .unwrap()
        .is_none());
    assert_eq!(
        engine
            .conn
            .query_row("SELECT COUNT(*) FROM image_revisions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );

    let duplicate = crate::pin::output::register_source_revision(
        &engine,
        &restored.source_png,
        None,
        &second_document,
        &second_render,
    )
    .unwrap();
    assert_eq!(duplicate, second_revision);

    let inconsistent = crate::screenshot::encode_png(&[1, 2, 3, 255, 4, 5, 6, 255], 2, 1).unwrap();
    assert!(crate::pin::output::register_source_revision(
        &engine,
        &restored.source_png,
        None,
        &second_document,
        &inconsistent,
    )
    .is_err());
    assert_eq!(
        engine
            .conn
            .query_row("SELECT COUNT(*) FROM image_revisions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );

    engine
        .conn
        .execute_batch(
            "CREATE TEMP TRIGGER fail_new_revision
             BEFORE INSERT ON image_revisions
             BEGIN SELECT RAISE(ABORT, 'fixture revision failure'); END;",
        )
        .unwrap();
    let other_source =
        crate::screenshot::encode_png(&[80, 70, 60, 255, 50, 40, 30, 255], 2, 1).unwrap();
    let other_document = editable_document(5);
    let other_render =
        crate::pin::output::render_document(&other_source, Some(&other_document)).unwrap();
    assert!(crate::pin::output::register_source_revision(
        &engine,
        &other_source,
        None,
        &other_document,
        &other_render,
    )
    .is_err());
    engine
        .conn
        .execute_batch("DROP TRIGGER fail_new_revision")
        .unwrap();
    assert_eq!(
        engine
            .conn
            .query_row("SELECT COUNT(*) FROM image_assets", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1,
        "修订事务失败不能留下孤儿根资产"
    );

    engine.clear_history().unwrap();
    assert!(engine.get_clip_by_id(root.id).is_err());
    assert!(engine.get_clip_by_id(first_clip.id).is_err());
    assert!(engine.get_clip_by_id(second_clip.id).is_err());
    assert_eq!(
        engine
            .get_image_revision_by_rendered_hash(&first_hash)
            .unwrap()
            .unwrap()
            .source_png,
        restored.source_png
    );
    assert_eq!(
        engine
            .conn
            .query_row("SELECT COUNT(*) FROM image_assets", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1,
        "历史清理不能回收仍被修订引用的根图"
    );
}

#[test]
fn image_revision_links_survive_database_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("revision.sqlite");
    let source =
        crate::screenshot::encode_png(&[10, 20, 30, 255, 200, 180, 160, 255], 2, 1).unwrap();
    let document = editable_document(25);
    let rendered = crate::pin::output::render_document(&source, Some(&document)).unwrap();
    let rendered_hash = crate::clipboard_watcher::content::compute_hash(&rendered);
    let clip_id = {
        let engine = StorageEngine::new(&path).unwrap();
        crate::pin::output::register_source_revision(&engine, &source, None, &document, &rendered)
            .unwrap();
        engine
            .insert_clip(
                &ContentType::Image,
                None,
                None,
                Some(&rendered),
                &rendered_hash,
                rendered.len() as i64,
                false,
            )
            .unwrap()
            .id
    };
    let reopened = StorageEngine::new(&path).unwrap();
    let revision = reopened
        .get_image_revision_for_clip(clip_id)
        .unwrap()
        .unwrap();
    assert_eq!(revision.source_png, source);
    assert_eq!(revision.adjustments["brightness"], 25);
}

#[test]
fn image_revision_schema_migrates_an_existing_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.sqlite");
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE clips (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    content_type TEXT NOT NULL,
                    text_content TEXT,
                    html_content TEXT,
                    image_data BLOB,
                    content_hash TEXT NOT NULL UNIQUE,
                    is_favorite INTEGER DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    byte_size INTEGER NOT NULL
                );",
            )
            .unwrap();
    }
    let engine = StorageEngine::new(&path).unwrap();
    let has_asset_column: bool = engine
        .conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('clips') WHERE name='image_asset_id')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let foreign_keys: bool = engine
        .conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert!(has_asset_column);
    assert!(foreign_keys);
    for table in ["image_assets", "image_revisions", "clip_image_revisions"] {
        let exists: bool = engine
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                params![table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "缺少迁移表 {table}");
    }
}

fn translation_of<'a>(
    clip_id: Option<i64>,
    provider: &'a str,
    translated: &'a str,
) -> NewTranslation<'a> {
    NewTranslation {
        clip_id,
        provider,
        source_language: "en",
        target_language: "zh",
        source_text: "hello world",
        translated_text: translated,
    }
}

#[test]
fn translation_history_keeps_one_row_per_service_and_target() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clip = insert_text(&engine, "hello world", "hash_translated");

    engine
        .record_translation(&translation_of(Some(clip.id), "deepl", "你好世界"))
        .unwrap();
    // 同一条目、同一服务、同一目标语言重复翻译只更新那一条记录。
    engine
        .record_translation(&translation_of(Some(clip.id), "deepl", "你好，世界"))
        .unwrap();
    engine
        .record_translation(&translation_of(Some(clip.id), "google", "你好 世界"))
        .unwrap();
    // 目标语言不同即另一条记录。
    engine
        .record_translation(&NewTranslation {
            target_language: "ja",
            ..translation_of(Some(clip.id), "deepl", "こんにちは")
        })
        .unwrap();
    // 不来自剪贴板条目的翻译同样入库，clip_id 存 0。
    engine
        .record_translation(&translation_of(None, "deepl", "选区译文"))
        .unwrap();

    let entries = engine.translation_history(Some(clip.id), 50).unwrap();
    assert_eq!(entries.len(), 3);
    let deepl_zh = entries
        .iter()
        .find(|entry| entry.provider == "deepl" && entry.target_language == "zh")
        .unwrap();
    assert_eq!(deepl_zh.translated_text, "你好，世界");
    assert_eq!(deepl_zh.source_text, "hello world");

    let all = engine.translation_history(None, 50).unwrap();
    assert_eq!(all.len(), 4);
    assert!(all.iter().any(|entry| entry.clip_id == 0));
}

#[test]
fn deleting_a_clip_also_deletes_its_translations() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clip = insert_text(&engine, "hello world", "hash_deleted");
    engine
        .record_translation(&translation_of(Some(clip.id), "deepl", "你好世界"))
        .unwrap();
    engine
        .record_translation(&translation_of(None, "deepl", "选区译文"))
        .unwrap();

    engine.delete_clip(clip.id).unwrap();
    let remaining = engine.translation_history(None, 50).unwrap();
    // 条目的译文随条目一起消失，与条目无关的记录保留。
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].clip_id, 0);

    engine.clear_translation_history().unwrap();
    assert!(engine.translation_history(None, 50).unwrap().is_empty());
}

#[test]
fn clearing_clip_history_removes_the_translations_it_leaves_behind() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let plain = insert_text(&engine, "hello world", "hash_plain");
    let favorite = insert_text(&engine, "favorite text", "hash_favorite");
    engine.toggle_favorite(favorite.id).unwrap();
    engine
        .record_translation(&translation_of(Some(plain.id), "deepl", "你好世界"))
        .unwrap();
    engine
        .record_translation(&translation_of(Some(favorite.id), "deepl", "收藏译文"))
        .unwrap();

    engine.clear_history().unwrap();

    let remaining = engine.translation_history(None, 50).unwrap();
    // 收藏条目不会被清理，它的译文也应该留着。
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].clip_id, favorite.id);
}

#[cfg(unix)]
#[test]
fn file_database_and_wal_sidecars_are_private() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("创建临时目录失败");
    let db_path = directory.path().join("clips.db");
    {
        let connection = rusqlite::Connection::open(&db_path).expect("创建数据库文件失败");
        connection
            .execute("CREATE TABLE seed(value TEXT)", [])
            .expect("初始化数据库文件失败");
    }
    fs::set_permissions(&db_path, fs::Permissions::from_mode(0o644)).unwrap();

    let engine = StorageEngine::new(&db_path).expect("打开数据库失败");
    assert_eq!(
        fs::metadata(&db_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    insert_text(&engine, "private clipboard", "private-hash");
    for suffix in ["-wal", "-shm"] {
        let sidecar = db_path.with_file_name(format!("clips.db{suffix}"));
        assert!(sidecar.exists(), "SQLite sidecar 应存在: {suffix}");
        assert_eq!(
            fs::metadata(sidecar).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

/// 删除条目要么全成要么全不成：中途失败不能留下"搜得到但已不存在"的 FTS 幽灵行。
///
/// 用一个 BEFORE DELETE 触发器把 translation_history 的删除强行打断——那是
/// delete_clip 的最后一步，此时 FTS 行和主表行都已经删过了。没有事务的话
/// 这条记录会消失但依然能被搜索命中，且 rebuild_fts_once 只在 schema 版本
/// 变化时跑，索引不会自己长回来。
#[test]
fn failed_delete_rolls_back_fts_and_clip_row() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clip = insert_text(&engine, "atomic delete apple", "hash_atomic_delete");
    engine
        .conn
        .execute(
            "CREATE TRIGGER block_translation_delete BEFORE DELETE ON translation_history
             BEGIN SELECT RAISE(ABORT, 'boom'); END",
            [],
        )
        .unwrap();
    engine
        .record_translation(&translation_of(Some(clip.id), "libretranslate", "苹果"))
        .unwrap();

    assert!(engine.delete_clip(clip.id).is_err(), "触发器应让删除失败");

    // 主表行、FTS 索引、译文三者都必须完整回滚
    assert_eq!(engine.get_clips(None, false, 0, 10).unwrap().len(), 1);
    assert_eq!(
        engine.get_clips(Some("apple"), false, 0, 10).unwrap().len(),
        1,
        "FTS 行不能被单独删掉"
    );
    let translations: i64 = engine
        .conn
        .query_row("SELECT COUNT(*) FROM translation_history", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(translations, 1);
}

/// 同理：清理超限条目时中途失败也不能只删一半。
#[test]
fn failed_cleanup_rolls_back_every_deletion() {
    let engine = StorageEngine::new_in_memory().unwrap();
    for i in 0..4 {
        insert_text(
            &engine,
            &format!("cleanup banana {i}"),
            &format!("hash_cl_{i}"),
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let oldest = engine.get_clips(None, false, 0, 10).unwrap().pop().unwrap();
    engine
        .record_translation(&translation_of(Some(oldest.id), "libretranslate", "香蕉"))
        .unwrap();
    engine
        .conn
        .execute(
            "CREATE TRIGGER block_translation_delete BEFORE DELETE ON translation_history
             BEGIN SELECT RAISE(ABORT, 'boom'); END",
            [],
        )
        .unwrap();

    assert!(engine.cleanup_old_entries(2).is_err(), "触发器应让清理失败");

    assert_eq!(engine.get_clips(None, false, 0, 10).unwrap().len(), 4);
    assert_eq!(
        engine
            .get_clips(Some("banana"), false, 0, 10)
            .unwrap()
            .len(),
        4,
        "FTS 行不能被单独删掉"
    );
}

fn url_meta(url: String) -> UrlMeta {
    UrlMeta {
        url,
        title: Some("title".to_string()),
        description: Some("description".to_string()),
        favicon: Some("https://example.com/favicon.ico".to_string()),
        site_name: Some("example".to_string()),
    }
}

#[test]
fn url_meta_cache_discards_expired_rows_when_writing() {
    let engine = StorageEngine::new_in_memory().unwrap();
    engine
        .conn
        .execute(
            "INSERT INTO url_meta_cache
                (url, title, description, favicon, site_name, fetched_at)
             VALUES (?1, NULL, NULL, NULL, NULL, ?2)",
            params![
                "https://expired.example",
                now_secs() - URL_META_TTL_SECS - 1
            ],
        )
        .unwrap();

    engine
        .set_url_meta(&url_meta("https://fresh.example".to_string()))
        .unwrap();

    let count: i64 = engine
        .conn
        .query_row("SELECT COUNT(*) FROM url_meta_cache", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert!(engine
        .get_url_meta("https://fresh.example")
        .unwrap()
        .is_some());
}

#[test]
fn url_meta_cache_keeps_only_the_newest_bounded_set() {
    let engine = StorageEngine::new_in_memory().unwrap();
    for index in 0..MAX_URL_META_ENTRIES + 8 {
        engine
            .set_url_meta(&url_meta(format!("https://example.com/{index}")))
            .unwrap();
    }

    let count: i64 = engine
        .conn
        .query_row("SELECT COUNT(*) FROM url_meta_cache", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, MAX_URL_META_ENTRIES);
    assert!(engine
        .get_url_meta("https://example.com/519")
        .unwrap()
        .is_some());
}

#[test]
fn unlimited_history_preserves_new_old_favorites_search_and_translations() {
    let engine = StorageEngine::new_in_memory().unwrap();
    assert!(engine.cleanup_old_entries(0).unwrap().is_empty());
    for index in 0..5 {
        let text = format!("unlimited needle {index}");
        let clip = insert_text(&engine, &text, &text);
        if index < 2 {
            engine.toggle_favorite(clip.id).unwrap();
        }
        engine
            .record_translation(&NewTranslation {
                clip_id: Some(clip.id),
                provider: "test",
                source_language: "en",
                target_language: "zh",
                source_text: &text,
                translated_text: "译文",
            })
            .unwrap();
        assert!(engine.cleanup_old_entries(0).unwrap().is_empty());
        insert_text(&engine, &text, &text);
        assert!(engine.cleanup_old_entries(0).unwrap().is_empty());
        assert_eq!(
            engine.translation_history(Some(clip.id), 10).unwrap().len(),
            1
        );
        assert_eq!(
            engine
                .get_clips(Some("needle"), false, 0, 10)
                .unwrap()
                .len(),
            index + 1
        );
    }
    assert_eq!(engine.get_clips(None, false, 0, 10).unwrap().len(), 5);
    assert_eq!(engine.cleanup_old_entries(1).unwrap().len(), 2);
    assert_eq!(engine.get_clips(None, false, 0, 10).unwrap().len(), 3);
    assert_eq!(engine.translation_history(None, 10).unwrap().len(), 3);
    assert!(engine.cleanup_old_entries(9).unwrap().is_empty());
}

#[test]
fn same_second_insert_touch_dedup_search_and_cleanup_share_usage_order() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let a = insert_text(&engine, "needle alpha", "a");
    let b = insert_text(&engine, "needle beta", "b");
    let c = insert_text(&engine, "needle gamma", "c");
    engine
        .conn
        .execute("UPDATE clips SET created_at = 123", [])
        .unwrap();
    let ids = |query, favorite| {
        engine
            .get_clips(query, favorite, 0, 10)
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(None, false), vec![c.id, b.id, a.id]);
    engine.touch_clip(a.id).unwrap();
    engine
        .conn
        .execute("UPDATE clips SET created_at = 123", [])
        .unwrap();
    assert_eq!(ids(None, false), vec![a.id, c.id, b.id]);
    insert_text(&engine, "needle beta", "b");
    engine
        .conn
        .execute("UPDATE clips SET created_at = 123", [])
        .unwrap();
    for query in [None, Some("ne"), Some("needle"), Some("needle b")] {
        let expected = if query == Some("needle b") {
            vec![b.id]
        } else {
            vec![b.id, a.id, c.id]
        };
        assert_eq!(ids(query, false), expected);
    }
    engine.toggle_favorite(a.id).unwrap();
    engine.toggle_favorite(b.id).unwrap();
    assert_eq!(ids(Some("needle"), true), vec![b.id, a.id]);
    engine.toggle_favorite(a.id).unwrap();
    engine.toggle_favorite(b.id).unwrap();
    assert_eq!(engine.cleanup_old_entries(2).unwrap(), vec![c.id]);
    assert_eq!(ids(None, false), vec![b.id, a.id]);
}

#[test]
fn use_order_migrates_legacy_rows_and_survives_reopen_and_clear() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy-use-order.db");
    let engine = StorageEngine {
        conn: Connection::open(&path).unwrap(),
    };
    engine
        .conn
        .execute_batch(
            "CREATE TABLE clips (
        id INTEGER PRIMARY KEY AUTOINCREMENT, content_type TEXT NOT NULL, text_content TEXT,
        html_content TEXT, image_data BLOB, content_hash TEXT NOT NULL UNIQUE,
        is_favorite INTEGER DEFAULT 0, created_at INTEGER NOT NULL, byte_size INTEGER NOT NULL);
        INSERT INTO clips VALUES (1, 'text', 'one', NULL, NULL, 'one', 0, 100, 3);
        INSERT INTO clips VALUES (2, 'text', 'two', NULL, NULL, 'two', 0, 90, 3);
        INSERT INTO clips VALUES (3, 'text', 'three', NULL, NULL, 'three', 0, 100, 5);",
        )
        .unwrap();
    engine.init_tables().unwrap();
    assert_eq!(
        engine
            .get_clips(None, false, 0, 10)
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![3, 1, 2]
    );
    engine.touch_clip(2).unwrap();
    drop(engine);
    // 真正关闭并重新打开磁盘数据库，不能用同一 Connection 的重复初始化代替持久性验证。
    let engine = StorageEngine::new(&path).unwrap();
    assert_eq!(engine.get_clips(None, false, 0, 10).unwrap()[0].id, 2);
    let order_before: i64 = engine
        .conn
        .query_row("SELECT MAX(use_order) FROM clips", [], |r| r.get(0))
        .unwrap();
    assert_eq!(order_before, 4);
    // migration 和 use_order 不篡改历史的秒级时间。
    assert_eq!(engine.get_clip_by_id(1).unwrap().created_at, 100);
    engine.clear_history().unwrap();
    drop(engine);
    let engine = StorageEngine::new(&path).unwrap();
    let new = insert_text(&engine, "new", "new");
    let order_after: i64 = engine
        .conn
        .query_row("SELECT use_order FROM clips WHERE id=?", [new.id], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(order_after > order_before);
}

#[test]
fn unlimited_history_can_shrink_beyond_the_sqlite_parameter_limit() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let favorite = insert_text(&engine, "favorite needle", "favorite");
    engine.toggle_favorite(favorite.id).unwrap();
    let mut ids = Vec::with_capacity(33_000);
    for index in 0..33_000 {
        let text = format!("bulk needle {index}");
        ids.push(insert_text(&engine, &text, &text).id);
    }
    // 覆盖首块、中间块、末块，以及不应删除的收藏和保留条目。
    for id in [favorite.id, ids[0], ids[750], ids[32_899], ids[32_999]] {
        engine
            .record_translation(&translation_of(Some(id), "deepl", "synthetic"))
            .unwrap();
    }
    assert!(engine.cleanup_old_entries(0).unwrap().is_empty());
    let removed = engine.cleanup_old_entries(100).unwrap();
    assert_eq!(removed, ids[..32_900]);
    let remaining = engine.get_clips(None, false, 0, 40_000).unwrap();
    assert_eq!(remaining.len(), 101);
    assert!(engine.get_clip_by_id(favorite.id).unwrap().is_favorite);
    assert_eq!(remaining[0].id, ids[32_999]);
    assert_eq!(
        engine
            .get_clips(Some("needle"), false, 0, 40_000)
            .unwrap()
            .len(),
        101
    );
    let translations = engine.translation_history(None, 100).unwrap();
    assert_eq!(translations.len(), 2);
    assert!(translations
        .iter()
        .all(|entry| [favorite.id, ids[32_999]].contains(&entry.clip_id)));
    engine
        .conn
        .execute(
            "INSERT INTO clips_fts(clips_fts, rank) VALUES ('integrity-check', 1)",
            [],
        )
        .unwrap();
}

#[test]
fn cleanup_and_sensitive_purge_roll_back_across_delete_chunks() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let mut ids = Vec::new();
    for index in 0..1_105 {
        let text = format!("rollback needle {index}");
        ids.push(insert_text(&engine, &text, &text).id);
    }
    engine.toggle_favorite(ids[1_104]).unwrap();
    for id in [ids[0], ids[750], ids[1_104]] {
        engine
            .record_translation(&translation_of(Some(id), "deepl", "synthetic"))
            .unwrap();
    }
    // id 751 位于第二块；首块已执行 DELETE 后才注入错误，必须回滚整个外层事务。
    engine.conn.execute_batch("CREATE TRIGGER fail_second_chunk BEFORE DELETE ON clips WHEN OLD.id = 751 BEGIN SELECT RAISE(ABORT, 'second chunk failed'); END;").unwrap();
    engine
        .conn
        .execute("UPDATE clips SET is_sensitive = 1, created_at = 0", [])
        .unwrap();
    for result in [
        engine.cleanup_old_entries(100),
        engine.purge_expired_sensitive(1),
    ] {
        assert!(result.is_err());
        assert_eq!(
            engine.get_clips(None, false, 0, 2_000).unwrap().len(),
            1_105
        );
        assert_eq!(
            engine
                .get_clips(Some("needle"), false, 0, 2_000)
                .unwrap()
                .len(),
            1_105
        );
        assert_eq!(engine.translation_history(None, 100).unwrap().len(), 3);
        engine
            .conn
            .execute(
                "INSERT INTO clips_fts(clips_fts, rank) VALUES ('integrity-check', 1)",
                [],
            )
            .unwrap();
    }
    engine
        .conn
        .execute_batch("DROP TRIGGER fail_second_chunk;")
        .unwrap();
    assert_eq!(engine.purge_expired_sensitive(1).unwrap().len(), 1_104);
    assert_eq!(
        engine.get_clips(Some("needle"), false, 0, 2_000).unwrap()[0].id,
        ids[1_104]
    );
    assert_eq!(
        engine.translation_history(None, 100).unwrap()[0].clip_id,
        ids[1_104]
    );
}

#[test]
fn use_order_and_clip_update_roll_back_together_on_failure() {
    let engine = StorageEngine::new_in_memory().unwrap();
    let clip = insert_text(&engine, "retained", "retained");
    engine.conn.execute_batch("CREATE TRIGGER reject_touch BEFORE UPDATE ON clips BEGIN SELECT RAISE(ABORT, 'busy'); END;").unwrap();
    assert!(engine.touch_clip(clip.id).is_err());
    assert!(engine
        .insert_clip(
            &ContentType::Text,
            Some("retained"),
            None,
            None,
            "retained",
            8,
            false
        )
        .is_err());
    let order: i64 = engine
        .conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM schema_meta WHERE key='clip_use_order'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(order, 1);
    assert!(engine.touch_clip(999).is_err());
    let order: i64 = engine
        .conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM schema_meta WHERE key='clip_use_order'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(order, 1);
}

#[test]
fn zero_and_positive_history_limits_leave_all_favorites_intact() {
    let engine = StorageEngine::new_in_memory().unwrap();
    for index in 0..4 {
        let clip = insert_text(&engine, &index.to_string(), &index.to_string());
        engine.toggle_favorite(clip.id).unwrap();
    }
    for limit in [0, 1, 2, 10] {
        assert!(engine.cleanup_old_entries(limit).unwrap().is_empty());
    }
    assert_eq!(engine.get_clips(None, true, 0, 10).unwrap().len(), 4);
}
