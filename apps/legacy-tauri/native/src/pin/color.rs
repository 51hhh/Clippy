use serde::Serialize;

/// 颜色 Pin 交给前端的已规范化 RGBA 值。
///
/// 原始文本始终留在 `PinPayload::text`；这里的 canonical 只作为安全、确定的渲染值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PinColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
    pub canonical: String,
}

impl PinColor {
    fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            red: r,
            green: g,
            blue: b,
            alpha: a,
            canonical: format!("#{r:02x}{g:02x}{b:02x}{a:02x}"),
        }
    }
}

/// 解析颜色 Pin 支持的严格、依赖无关语法。
///
/// 输入先受 128 UTF-8 bytes、ASCII 和单行控制字符限制，再仅去掉最外层 space/tab。
/// 任何不支持的形式都静默返回 `None`，由调用方保留为普通文本。
pub(crate) fn parse_pin_color(text: &str) -> Option<PinColor> {
    if text.len() > 128
        || !text.is_ascii()
        || text
            .bytes()
            .any(|byte| byte.is_ascii_control() && byte != b'\t')
    {
        return None;
    }
    let candidate = text.trim_matches([' ', '\t']);
    if candidate.is_empty() {
        return None;
    }

    parse_hex(candidate)
        .or_else(|| parse_function(candidate))
        .or_else(|| parse_integer_fallback(candidate))
}

fn parse_hex(candidate: &str) -> Option<PinColor> {
    let digits = candidate.strip_prefix('#')?;
    let values = match digits.len() {
        3 => [
            hex_pair(&digits[0..1])?,
            hex_pair(&digits[1..2])?,
            hex_pair(&digits[2..3])?,
            255,
        ],
        4 => [
            hex_pair(&digits[0..1])?,
            hex_pair(&digits[1..2])?,
            hex_pair(&digits[2..3])?,
            hex_pair(&digits[3..4])?,
        ],
        6 => [
            hex_byte(&digits[0..2])?,
            hex_byte(&digits[2..4])?,
            hex_byte(&digits[4..6])?,
            255,
        ],
        8 => [
            hex_byte(&digits[0..2])?,
            hex_byte(&digits[2..4])?,
            hex_byte(&digits[4..6])?,
            hex_byte(&digits[6..8])?,
        ],
        _ => return None,
    };
    Some(PinColor::new(values[0], values[1], values[2], values[3]))
}

fn hex_pair(digit: &str) -> Option<u8> {
    Some(hex_digit(digit.as_bytes()[0])? * 17)
}

fn hex_byte(digits: &str) -> Option<u8> {
    let bytes = digits.as_bytes();
    Some(hex_digit(bytes[0])? * 16 + hex_digit(bytes[1])?)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_function(candidate: &str) -> Option<PinColor> {
    let (components, is_rgba) = if candidate.len() >= 5
        && candidate[..4].eq_ignore_ascii_case("rgb(")
        && candidate.ends_with(')')
    {
        (&candidate[4..candidate.len() - 1], false)
    } else if candidate.len() >= 6
        && candidate[..5].eq_ignore_ascii_case("rgba(")
        && candidate.ends_with(')')
    {
        (&candidate[5..candidate.len() - 1], true)
    } else {
        return None;
    };
    parse_components(components, is_rgba, true)
}

fn parse_integer_fallback(candidate: &str) -> Option<PinColor> {
    parse_components(candidate, false, false)
}

fn parse_components(
    components: &str,
    function_is_rgba: bool,
    function_form: bool,
) -> Option<PinColor> {
    let values: Vec<&str> = components
        .split(',')
        .map(|value| value.trim_matches([' ', '\t']))
        .collect();
    let expected_arity = if function_form {
        if function_is_rgba {
            4
        } else {
            3
        }
    } else if values.len() == 3 || values.len() == 4 {
        values.len()
    } else {
        return None;
    };
    if values.len() != expected_arity || values.iter().any(|value| value.is_empty()) {
        return None;
    }

    let r = parse_byte(values[0])?;
    let g = parse_byte(values[1])?;
    let b = parse_byte(values[2])?;
    let a = if values.len() == 4 {
        if function_form {
            parse_function_alpha(values[3])?
        } else {
            parse_byte(values[3])?
        }
    } else {
        255
    };
    Some(PinColor::new(r, g, b, a))
}

fn parse_byte(value: &str) -> Option<u8> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse::<u8>().ok())
        .flatten()
}

fn parse_function_alpha(value: &str) -> Option<u8> {
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction)) if !fraction.is_empty() && !fraction.contains('.') => {
            (whole, Some(fraction))
        }
        Some(_) => return None,
        None => (value, None),
    };
    if !matches!(whole, "0" | "1")
        || fraction.is_some_and(|fraction| !fraction.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    if whole == "1" && fraction.is_some_and(|fraction| fraction.bytes().any(|byte| byte != b'0')) {
        return None;
    }

    match (whole, fraction) {
        ("1", _) => Some(255),
        ("0", None) => Some(0),
        ("0", Some(fraction)) => Some(round_fraction_to_alpha(fraction)),
        _ => None,
    }
}

/// 把 `0.<fraction>` 精确映射到 8-bit alpha，避免长小数先变成 f64 后跨过舍入边界。
fn round_fraction_to_alpha(fraction: &str) -> u8 {
    let mut product = Vec::with_capacity(fraction.len() + 3);
    let mut carry = 0_u16;
    for digit in fraction.bytes().rev() {
        let value = u16::from(digit - b'0') * 255 + carry;
        product.push((value % 10) as u8);
        carry = value / 10;
    }
    while carry > 0 {
        product.push((carry % 10) as u8);
        carry /= 10;
    }
    product.reverse();

    let decimal_places = fraction.len();
    let (integer, first_fractional_digit) = if product.len() > decimal_places {
        let split = product.len() - decimal_places;
        let integer = product[..split]
            .iter()
            .fold(0_u8, |value, digit| value * 10 + *digit);
        (integer, product[split])
    } else {
        let first_fractional_digit = if product.len() == decimal_places {
            product[0]
        } else {
            0
        };
        (0, first_fractional_digit)
    };
    integer + u8::from(first_fractional_digit >= 5)
}

#[cfg(test)]
mod tests {
    use super::parse_pin_color;

    #[test]
    fn parses_every_supported_form_to_lowercase_canonical_rgba() {
        let cases = [
            ("#AbC", (170, 187, 204, 255), "#aabbccff"),
            ("#AbCd", (170, 187, 204, 221), "#aabbccdd"),
            ("#A1b2C3", (161, 178, 195, 255), "#a1b2c3ff"),
            ("#A1b2C3d4", (161, 178, 195, 212), "#a1b2c3d4"),
            ("RGB(1,2,3)", (1, 2, 3, 255), "#010203ff"),
            ("RgBa(1,2,3,0.5)", (1, 2, 3, 128), "#01020380"),
            ("1,2,3", (1, 2, 3, 255), "#010203ff"),
            ("1,2,3,4", (1, 2, 3, 4), "#01020304"),
            ("\t #0f08 \t", (0, 255, 0, 136), "#00ff0088"),
        ];

        for (input, rgba, canonical) in cases {
            let color = parse_pin_color(input).unwrap_or_else(|| panic!("应解析: {input}"));
            assert_eq!(
                (color.red, color.green, color.blue, color.alpha),
                rgba,
                "{input}"
            );
            assert_eq!(color.canonical, canonical, "{input}");
        }
    }

    #[test]
    fn rounds_function_alpha_at_its_boundaries() {
        let cases = [
            ("rgba(0,0,0,0)", 0),
            ("rgba(0,0,0,0.001)", 0),
            ("rgba(0,0,0,0.5)", 128),
            ("rgba(0,0,0,0.4999999999999999999999)", 127),
            ("rgba(0,0,0,0.5000000000000000000000)", 128),
            ("rgba(0,0,0,0.998)", 254),
            ("rgba(0,0,0,1)", 255),
            ("rgba(0,0,0,1.000)", 255),
        ];

        for (input, alpha) in cases {
            assert_eq!(parse_pin_color(input).map(|color| color.alpha), Some(alpha));
        }
    }

    #[test]
    fn rejects_out_of_contract_input_without_panicking() {
        let too_long = "#".repeat(129);
        let cases = [
            "",
            "\n#fff",
            "#fff\r",
            "#fff\0",
            "#fff\u{007f}",
            "#fff😀",
            "#ff",
            "#fffff",
            "#ggg",
            "rgb(1,2)",
            "rgb(1,2,3,4)",
            "rgba(1,2,3)",
            "rgba(1,2,3,4,5)",
            "rgb(1,,3)",
            "1,2",
            "1,2,3,4,5",
            "1,,3",
            "-1,2,3",
            "+1,2,3",
            "1.5,2,3",
            "1e2,2,3",
            "rgb(100%,2,3)",
            "rgba(1,2,3,.5)",
            "rgba(1,2,3,0.)",
            "rgba(1,2,3,1.01)",
            "rgba(1,2,3,1e0)",
            "rgba(1,2,3,50%)",
            "256,2,3",
            "rgb(1,2,256)",
            "rgb(1,2,3\n)",
        ];

        for input in cases.into_iter().chain(std::iter::once(too_long.as_str())) {
            assert!(parse_pin_color(input).is_none(), "应拒绝: {input:?}");
        }
    }

    #[test]
    fn allows_space_and_tab_around_comma_components() {
        let cases = [
            ("rgb( 1, 2, 3 )", "#010203ff"),
            ("rgba(\t1\t, 2 ,\t3, 0.5\t)", "#01020380"),
            ("\t 1 ,\t2, 3 , 4 \t", "#01020304"),
        ];

        for (input, canonical) in cases {
            assert_eq!(
                parse_pin_color(input).map(|color| color.canonical),
                Some(canonical.to_string()),
                "{input}"
            );
        }
    }
}
