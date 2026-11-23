use super::*;
use std::borrow::Cow;

struct Reader {
    html: Option<String>,
    text: Option<String>,
    image: bool,
    replacement: Option<(Option<String>, Option<String>)>,
    calls: Vec<&'static str>,
}

impl Reader {
    fn rich() -> Self {
        Self {
            html: Some("<b>payload A</b>".into()),
            text: Some("custom alternative A".into()),
            image: false,
            replacement: None,
            calls: Vec::new(),
        }
    }

    fn apply_replacement(&mut self) {
        if let Some((html, text)) = self.replacement.take() {
            self.html = html;
            self.text = text;
        }
    }
}

impl SnapshotReader for Reader {
    fn html(&mut self) -> Result<String, arboard::Error> {
        self.calls.push("html");
        let result = self.html.clone().ok_or(arboard::Error::ContentNotAvailable);
        // 原协议在 HTML guard 结束后，外部复制可在第二次读取前完成。
        self.apply_replacement();
        result
    }

    fn text(&mut self) -> Result<String, arboard::Error> {
        self.calls.push("text");
        self.text.clone().ok_or(arboard::Error::ContentNotAvailable)
    }

    fn image(&mut self) -> Result<arboard::ImageData<'static>, arboard::Error> {
        self.calls.push("image");
        if self.image {
            Ok(arboard::ImageData {
                width: 1,
                height: 1,
                bytes: Cow::Owned(vec![1, 2, 3, 255]),
            })
        } else {
            Err(arboard::Error::ContentNotAvailable)
        }
    }

    // paired adapter 的受控合同；原生 guard 生命周期另由生产 vendor helper 测试证明。
    fn html_with_text(&mut self) -> Result<(String, Option<String>), arboard::Error> {
        self.calls.push("paired");
        let result = self
            .html
            .clone()
            .map(|html| (html, self.text.clone()))
            .ok_or(arboard::Error::ContentNotAvailable);
        self.apply_replacement();
        result
    }
}

fn pair(snapshot: Option<ClipboardSnapshot>) -> (String, String) {
    match snapshot.unwrap() {
        ClipboardSnapshot::Html { html, text } => (html, text),
        _ => panic!("应保留本次 HTML 快照"),
    }
}

#[test]
fn external_rich_copy_between_reads_cannot_mix_old_html_and_new_alternative() {
    let mut reader = Reader::rich();
    reader.replacement = Some((
        Some("<i>payload B</i>".into()),
        Some("alternative B".into()),
    ));
    assert_eq!(
        pair(ClipboardSnapshot::read_with(&mut reader)),
        ("<b>payload A</b>".into(), "custom alternative A".into())
    );
    assert_eq!(reader.calls, ["paired"]);
    assert_eq!(reader.text.as_deref(), Some("alternative B"));
}

#[test]
fn external_plain_copy_between_reads_cannot_attach_new_text_to_old_html() {
    let mut reader = Reader::rich();
    reader.replacement = Some((None, Some("plain B".into())));
    assert_eq!(
        pair(ClipboardSnapshot::read_with(&mut reader)),
        ("<b>payload A</b>".into(), "custom alternative A".into())
    );
    assert_eq!(reader.calls, ["paired"]);
}

#[test]
fn external_copy_removing_text_cannot_replace_a_valid_alternative_with_stripped_html() {
    let mut reader = Reader::rich();
    reader.replacement = Some((None, None));
    assert_eq!(
        pair(ClipboardSnapshot::read_with(&mut reader)),
        ("<b>payload A</b>".into(), "custom alternative A".into())
    );
}

#[test]
fn production_prepare_and_temporary_storage_do_not_persist_a_mixed_pair() {
    let mut reader = Reader::rich();
    reader.replacement = Some((None, Some("plain B".into())));
    let snapshot = ClipboardSnapshot::read_with(&mut reader).unwrap();
    let mut state = PollState::default();
    let prepared = prepare_snapshot(snapshot, &[], &mut state, &mut None, Instant::now()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let storage = StorageEngine::new(&directory.path().join("snapshot.db")).unwrap();
    let clip = storage
        .insert_clip(
            &prepared.kind,
            prepared.text.as_deref(),
            prepared.html.as_deref(),
            prepared.png.as_deref(),
            &prepared.hash,
            prepared.byte_size,
            prepared.sensitive,
        )
        .unwrap();
    assert_eq!(clip.html_content.as_deref(), Some("<b>payload A</b>"));
    assert_eq!(clip.text_content.as_deref(), Some("custom alternative A"));
}

#[test]
fn stable_html_keeps_a_custom_alternative_and_successful_empty_alternative() {
    let mut reader = Reader::rich();
    assert_eq!(
        pair(ClipboardSnapshot::read_with(&mut reader)),
        ("<b>payload A</b>".into(), "custom alternative A".into())
    );
    reader.text = Some(String::new());
    assert_eq!(
        pair(ClipboardSnapshot::read_with(&mut reader)),
        ("<b>payload A</b>".into(), String::new())
    );
}

#[test]
fn unavailable_alternative_is_derived_from_the_same_html() {
    let mut reader = Reader::rich();
    reader.text = None;
    assert_eq!(
        pair(ClipboardSnapshot::read_with(&mut reader)),
        ("<b>payload A</b>".into(), "payload A".into())
    );
}

#[test]
fn absent_or_empty_html_falls_back_to_text_then_image_then_unavailable() {
    for html in [None, Some(String::new())] {
        let mut reader = Reader::rich();
        reader.html = html;
        assert!(
            matches!(ClipboardSnapshot::read_with(&mut reader), Some(ClipboardSnapshot::Text(text)) if text == "custom alternative A")
        );
        reader.text = Some(String::new());
        reader.image = true;
        assert!(
            matches!(ClipboardSnapshot::read_with(&mut reader), Some(ClipboardSnapshot::Image(image)) if image.bytes.as_ref() == [1, 2, 3, 255])
        );
        reader.text = None;
        reader.image = false;
        assert!(ClipboardSnapshot::read_with(&mut reader).is_none());
    }
}

#[test]
fn paired_html_keeps_existing_suppression_and_hash_contract() {
    let mut reader = Reader::rich();
    let snapshot = ClipboardSnapshot::read_with(&mut reader).unwrap();
    let mut state = PollState::default();
    let hash = compute_hash(b"<b>payload A</b>");
    assert!(prepare_snapshot(
        snapshot,
        std::slice::from_ref(&hash),
        &mut state,
        &mut None,
        Instant::now()
    )
    .is_none());
    let snapshot = ClipboardSnapshot::read_with(&mut reader).unwrap();
    assert!(prepare_snapshot(snapshot, &[], &mut state, &mut None, Instant::now()).is_none());
    state.reset();
    let snapshot = ClipboardSnapshot::read_with(&mut reader).unwrap();
    let prepared = prepare_snapshot(snapshot, &[], &mut state, &mut None, Instant::now()).unwrap();
    assert_eq!(prepared.hash, hash);
    assert_eq!(prepared.text.as_deref(), Some("custom alternative A"));
}
