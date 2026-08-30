use super::*;
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
