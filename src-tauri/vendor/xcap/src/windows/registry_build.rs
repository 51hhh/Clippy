//! Windows 构建号读取的有界 UTF-16 合同，测试不访问系统注册表。

pub(super) fn query_build_number(read: impl FnOnce(&mut [u16], &mut u32) -> bool) -> u32 {
    // RegGetValueW 的长度单位是字节；所有元素先初始化，不能用字节数调用 Vec::set_len。
    let mut buffer = [0u16; 1024];
    let mut byte_count = std::mem::size_of_val(&buffer) as u32;
    if !read(&mut buffer, &mut byte_count) {
        return 0;
    }
    let byte_count = byte_count as usize;
    if !byte_count.is_multiple_of(std::mem::size_of::<u16>()) {
        return 0;
    }
    let Some(value) = buffer.get(..byte_count / std::mem::size_of::<u16>()) else {
        return 0;
    };
    // 终止符必须位于 API 返回范围内，不能借用已清零但未写入的尾部。
    let Some(end) = value.iter().position(|&unit| unit == 0) else {
        return 0;
    };
    String::from_utf16(&value[..end])
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::query_build_number;

    fn write_value(buffer: &mut [u16], byte_count: &mut u32, value: &[u16]) {
        buffer[..value.len()].copy_from_slice(value);
        *byte_count = std::mem::size_of_val(value) as u32;
    }

    #[test]
    fn advertises_bytes_for_a_fully_initialized_buffer() {
        assert_eq!(
            query_build_number(|buffer, byte_count| {
                assert_eq!(*byte_count, 2048);
                assert_eq!(std::mem::size_of_val(buffer), 2048);
                assert!(buffer.iter().all(|&unit| unit == 0));
                write_value(buffer, byte_count, &[50, 54, 49, 48, 48, 0]);
                true
            }),
            26100
        );
    }

    #[test]
    fn parses_windows_build_samples_without_reading_tail() {
        for (text, expected) in [
            ("10240", 10240),
            ("19041", 19041),
            ("22000", 22000),
            ("26100", 26100),
        ] {
            let value: Vec<_> = text.encode_utf16().chain([0]).collect();
            assert_eq!(
                query_build_number(|buffer, byte_count| {
                    buffer.fill(0xd800);
                    write_value(buffer, byte_count, &value);
                    true
                }),
                expected
            );
        }
    }

    #[test]
    fn short_return_cannot_borrow_a_terminator_from_tail() {
        for returned_bytes in [0, 10] {
            assert_eq!(
                query_build_number(|buffer, byte_count| {
                    write_value(buffer, byte_count, &[50, 54, 49, 48, 48, 0]);
                    *byte_count = returned_bytes;
                    true
                }),
                0
            );
        }
    }

    #[test]
    fn odd_return_size_is_rejected() {
        assert_eq!(
            query_build_number(|buffer, byte_count| {
                write_value(buffer, byte_count, &[50, 54, 49, 48, 48, 0]);
                *byte_count = 11;
                true
            }),
            0
        );
    }

    #[test]
    fn oversized_return_size_is_rejected() {
        for returned_bytes in [2050, u32::MAX] {
            assert_eq!(
                query_build_number(|buffer, byte_count| {
                    write_value(buffer, byte_count, &[50, 54, 49, 48, 48, 0]);
                    *byte_count = returned_bytes;
                    true
                }),
                0
            );
        }
    }

    #[test]
    fn failed_query_never_parses_partial_write() {
        assert_eq!(
            query_build_number(|buffer, byte_count| {
                write_value(buffer, byte_count, &[50, 54, 49, 48, 48, 0]);
                false
            }),
            0
        );
    }

    #[test]
    fn invalid_values_fall_back_to_zero() {
        for text in ["", "26100x", "4294967296"] {
            let value: Vec<_> = text.encode_utf16().chain([0]).collect();
            assert_eq!(
                query_build_number(|buffer, byte_count| {
                    write_value(buffer, byte_count, &value);
                    true
                }),
                0
            );
        }
        assert_eq!(
            query_build_number(|buffer, byte_count| {
                write_value(buffer, byte_count, &[0xd800, 0]);
                true
            }),
            0
        );
    }

    #[test]
    fn full_capacity_value_is_bounded_and_terminated() {
        assert_eq!(
            query_build_number(|buffer, byte_count| {
                buffer.fill(u16::from(b'0'));
                let end = buffer.len();
                buffer[end - 6..].copy_from_slice(&[50, 54, 49, 48, 48, 0]);
                *byte_count = std::mem::size_of_val(buffer) as u32;
                true
            }),
            26100
        );
    }
}
