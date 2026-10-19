//! Clippy 的 Windows CF_HTML 边界合同；所有解析回归都只操作离线字节。

use crate::common::Error;
use std::ops::Range;

/// 片段范围同时受实际字节长度与 UTF-8 字符边界约束。
fn fragment_range(data: &[u8]) -> Result<Range<usize>, Error> {
	let text = std::str::from_utf8(data).map_err(|_| Error::ConversionFailure)?;
	let mut start = None;
	let mut end = None;
	for line in text.split(['\r', '\n']).filter(|line| !line.is_empty()) {
		if line.starts_with('<') {
			break;
		}
		let Some((key, value)) = line.split_once(':') else { break };
		let slot = match key {
			"StartFragment" => &mut start,
			"EndFragment" => &mut end,
			_ => continue,
		};
		let value = value.trim();
		if slot.is_some() || value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
			return Err(Error::ConversionFailure);
		}
		*slot = Some(value.parse::<usize>().map_err(|_| Error::ConversionFailure)?);
		// 不再扫描片段正文，避免正文中的同名字符串覆盖已经解析的头字段。
		if start.is_some() && end.is_some() {
			break;
		}
	}
	let range = start.ok_or(Error::ConversionFailure)?..end.ok_or(Error::ConversionFailure)?;
	if text.get(range.clone()).is_none() {
		return Err(Error::ConversionFailure);
	}
	Ok(range)
}

pub(super) fn fragment(data: &[u8]) -> Result<&str, Error> {
	let range = fragment_range(data)?;
	let bytes = data.get(range).ok_or(Error::ConversionFailure)?;
	std::str::from_utf8(bytes).map_err(|_| Error::ConversionFailure)
}

/// 已打开的同一个 guard 覆盖 HTML 和替代文本；失败/展开由 Drop 释放。
pub(super) fn read_with_text_guard<G>(
	open: Result<G, Error>,
	read_html: impl FnOnce() -> Result<String, Error>,
	read_text: impl FnOnce() -> Result<String, Error>,
) -> Result<(String, Option<String>), Error> {
	let _guard = open?;
	let html = read_html()?;
	let text = if html.is_empty() { None } else { read_text().ok() };
	Ok((html, text))
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{cell::RefCell, rc::Rc};

	#[derive(Default)]
	struct GuardState {
		locked: bool,
		pending_copy: bool,
		current: &'static str,
		sequence: u32,
		drops: usize,
	}

	struct Guard(Rc<RefCell<GuardState>>);

	impl Guard {
		fn open(state: &Rc<RefCell<GuardState>>) -> Self {
			assert!(!state.borrow().locked);
			state.borrow_mut().locked = true;
			Self(state.clone())
		}
	}

	impl Drop for Guard {
		fn drop(&mut self) {
			let mut state = self.0.borrow_mut();
			state.locked = false;
			state.drops += 1;
			if state.pending_copy {
				state.pending_copy = false;
				state.current = "B";
			}
		}
	}

	#[test]
	fn paired_guard_blocks_copy_switch_even_with_zero_or_delayed_sequence() {
		for sequence in [0, 17] {
			let state = Rc::new(RefCell::new(GuardState {
				current: "A",
				sequence,
				..GuardState::default()
			}));
			let result = read_with_text_guard(
				Ok(Guard::open(&state)),
				|| {
					let mut source = state.borrow_mut();
					assert!(source.locked);
					source.pending_copy = true;
					if source.sequence != 0 {
						source.sequence += 1;
					}
					Ok(format!("<b>{}</b>", source.current))
				},
				|| {
					assert!(state.borrow().locked);
					Ok(state.borrow().current.into())
				},
			)
			.unwrap();
			assert_eq!(result, ("<b>A</b>".into(), Some("A".into())));
			assert!(!state.borrow().locked);
			assert_eq!(state.borrow().current, "B");
			assert_eq!(state.borrow().drops, 1);
		}
	}

	#[test]
	fn paired_open_failure_reads_neither_format() {
		assert!(matches!(
			read_with_text_guard::<Guard>(
				Err(Error::ClipboardOccupied),
				|| panic!("打开失败不得读取 HTML"),
				|| panic!("打开失败不得读取文本"),
			),
			Err(Error::ClipboardOccupied)
		));
	}

	#[test]
	fn paired_html_failure_releases_guard_without_reading_text() {
		let state = Rc::new(RefCell::new(GuardState::default()));
		assert!(matches!(
			read_with_text_guard(
				Ok(Guard::open(&state)),
				|| Err(Error::ConversionFailure),
				|| panic!("HTML 失败后交由 watcher 回退"),
			),
			Err(Error::ConversionFailure)
		));
		assert!(!state.borrow().locked);
		assert_eq!(state.borrow().drops, 1);
	}

	#[test]
	fn paired_empty_html_skips_alternative_and_releases_guard() {
		let state = Rc::new(RefCell::new(GuardState::default()));
		let result = read_with_text_guard(
			Ok(Guard::open(&state)),
			|| Ok(String::new()),
			|| panic!("空 HTML 不重复读取替代文本"),
		)
		.unwrap();
		assert_eq!(result, (String::new(), None));
		assert!(!state.borrow().locked);
		assert_eq!(state.borrow().drops, 1);
	}

	#[test]
	fn paired_alternative_failure_preserves_html_and_releases_guard() {
		let state = Rc::new(RefCell::new(GuardState::default()));
		let result = read_with_text_guard(
			Ok(Guard::open(&state)),
			|| Ok("<b>A</b>".into()),
			|| {
				assert!(state.borrow().locked);
				Err(Error::ContentNotAvailable)
			},
		)
		.unwrap();
		assert_eq!(result, ("<b>A</b>".into(), None));
		assert!(!state.borrow().locked);
		assert_eq!(state.borrow().drops, 1);
	}

	#[test]
	fn paired_empty_alternative_is_success_and_releases_guard() {
		let state = Rc::new(RefCell::new(GuardState::default()));
		let result = read_with_text_guard(
			Ok(Guard::open(&state)),
			|| Ok("<b>A</b>".into()),
			|| {
				assert!(state.borrow().locked);
				Ok(String::new())
			},
		)
		.unwrap();
		assert_eq!(result, ("<b>A</b>".into(), Some(String::new())));
		assert!(!state.borrow().locked);
		assert_eq!(state.borrow().drops, 1);
	}

	#[test]
	fn paired_read_unwind_releases_guard_for_the_next_reader() {
		let state = Rc::new(RefCell::new(GuardState::default()));
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			read_with_text_guard(
				Ok(Guard::open(&state)),
				|| panic!("受控读取展开"),
				|| panic!("不得继续"),
			)
		}));
		assert!(result.is_err());
		assert!(!state.borrow().locked);
		assert_eq!(state.borrow().drops, 1);
		drop(Guard::open(&state));
		assert_eq!(state.borrow().drops, 2);
	}

	fn fixture(fragment: &str, newline: &str) -> (Vec<u8>, Range<usize>) {
		let header = format!("Version:1.0{newline}StartHTML:-1{newline}EndHTML:-1{newline}SourceURL:https://example.invalid/test{newline}StartFragment:{:010}{newline}EndFragment:{:010}{newline}", 0, 0);
		let start = header.len();
		let end = start + fragment.len();
		let header = header
			.replace("StartFragment:0000000000", &format!("StartFragment:{start:010}"))
			.replace("EndFragment:0000000000", &format!("EndFragment:{end:010}"));
		(format!("{header}{fragment}\0").into_bytes(), start..end)
	}

	fn offsets(data: &[u8], start: &str, end: &str) -> Vec<u8> {
		let text = std::str::from_utf8(data).unwrap();
		text.lines()
			.map(|line| {
				if line.starts_with("StartFragment:") {
					format!("StartFragment:{start}")
				} else if line.starts_with("EndFragment:") {
					format!("EndFragment:{end}")
				} else {
					line.to_owned()
				}
			})
			.collect::<Vec<_>>()
			.join("\r\n")
			.into_bytes()
	}

	#[test]
	fn rejects_a_short_fragment_entirely_outside_the_buffer() {
		let (data, _) = fixture("<b>test</b>", "\r\n");
		let start = data.len() + 100;
		let malformed = offsets(&data, &format!("{start:010}"), &format!("{:010}", start + 3));
		assert!(fragment_range(&malformed).is_err());
	}

	#[test]
	fn rejects_an_end_past_the_buffer_even_with_a_small_length() {
		let (data, _) = fixture("<b>test</b>", "\r\n");
		let malformed =
			offsets(&data, &format!("{:010}", data.len() - 1), &format!("{:010}", data.len() + 1));
		assert!(fragment_range(&malformed).is_err());
	}

	#[test]
	fn rejects_reversed_offsets() {
		let (data, range) = fixture("<b>test</b>", "\r\n");
		assert!(fragment_range(&offsets(&data, &range.end.to_string(), &range.start.to_string()))
			.is_err());
	}

	#[test]
	fn actual_arboard_wrapping_preserves_unicode_fragment_bytes() {
		let fragment = "<b>中文 😀 &amp; café</b>\r\n";
		let wrapped = super::super::wrap_html(fragment);
		let range = fragment_range(wrapped.as_bytes()).unwrap();
		assert_eq!(wrapped.get(range).unwrap(), fragment);
		assert_eq!(super::fragment(wrapped.as_bytes()).unwrap(), fragment);
	}

	#[test]
	fn supports_all_header_newlines_and_optional_context() {
		for newline in ["\r\n", "\n", "\r"] {
			let (data, expected) = fixture("<p>中文 😀</p>", newline);
			assert_eq!(fragment_range(&data).unwrap(), expected);
			assert_eq!(fragment(&data).unwrap(), "<p>中文 😀</p>");
		}
	}

	#[test]
	fn rejects_missing_negative_nonnumeric_and_overflowed_offsets() {
		let (data, range) = fixture("test", "\r\n");
		for bad in ["-1", "not-a-number", "+1", "184467440737095516160"] {
			assert!(fragment_range(&offsets(&data, bad, &range.end.to_string())).is_err());
			assert!(fragment_range(&offsets(&data, &range.start.to_string(), bad)).is_err());
		}
		assert!(fragment_range(b"Version:1.0\r\n<html>test</html>").is_err());
		assert!(fragment_range(b"StartFragment:0000000000\r\n").is_err());
	}

	#[test]
	fn zero_offsets_are_valid_empty_ranges() {
		assert_eq!(
			fragment_range(b"StartFragment:0000000000\r\nEndFragment:0000000000\r\n").unwrap(),
			0..0
		);
	}

	#[test]
	fn rejects_offsets_inside_utf8_characters_and_invalid_utf8() {
		let (data, range) = fixture("中文 😀", "\r\n");
		let malformed =
			offsets(&data, &format!("{:010}", range.start + 1), &format!("{:010}", range.end));
		assert!(fragment_range(&malformed).is_err());
		assert!(fragment_range(b"StartFragment:0\r\nEndFragment:1\r\n\xff").is_err());
	}

	#[test]
	fn production_html_reader_uses_safe_bytes_and_fragment_parser() {
		let source = include_str!("../windows.rs");
		assert!(!source.contains("clipboard_win::raw::get_html"));
		assert!(source.contains("html::fragment(&out)"));
	}
}
