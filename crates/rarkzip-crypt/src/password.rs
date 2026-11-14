use std::borrow::Cow;

use zeroize::Zeroizing;

/// Trim exactly like the legacy desktop UI: remove UTF-16 units `<= ' '` from
/// both ends.
pub fn trim_java(input: &str) -> Cow<'_, str> {
    let units: Vec<u16> = input.encode_utf16().collect();
    let mut start = 0usize;
    let mut end = units.len();
    while start < end && units[start] <= b' ' as u16 {
        start += 1;
    }
    while start < end && units[end - 1] <= b' ' as u16 {
        end -= 1;
    }
    if start == 0 && end == units.len() {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(String::from_utf16_lossy(&units[start..end]))
    }
}

/// Expand a password into the byte string used by the compatible formats.
///
/// Printable ASCII characters are kept as-is. Every other UTF-16 code unit is
/// replaced by its decimal value, exactly like the legacy implementation,
/// including its handling of surrogate pairs (both units are expanded).
pub fn normalize(password: &str) -> Zeroizing<Vec<u8>> {
    let trimmed = trim_java(password);
    let units: Vec<u16> = trimmed.encode_utf16().collect();
    let mut out = String::with_capacity(trimmed.len());
    let mut index = 0usize;
    while index < units.len() {
        let cp = code_point_at(&units, index);
        if (32..=126).contains(&cp) {
            if let Some(ch) = char::from_u32(cp) {
                out.push(ch);
            }
        } else {
            out.push_str(&cp.to_string());
        }
        index += 1;
    }
    Zeroizing::new(out.into_bytes())
}

fn code_point_at(units: &[u16], index: usize) -> u32 {
    let first = units[index] as u32;
    if (0xD800..=0xDBFF).contains(&first) && index + 1 < units.len() {
        let second = units[index + 1] as u32;
        if (0xDC00..=0xDFFF).contains(&second) {
            return 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_unchanged_after_trim() {
        assert_eq!(&*normalize("  secret  "), b"secret");
    }

    #[test]
    fn non_ascii_is_expanded_to_decimal() {
        assert_eq!(&*normalize("é"), b"233");
        assert_eq!(&*normalize("中"), b"20013");
    }
}
