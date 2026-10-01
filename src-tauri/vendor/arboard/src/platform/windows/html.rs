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

#[cfg(test)]
mod tests {
	use super::*;

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
