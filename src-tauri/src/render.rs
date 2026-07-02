//! Heuristic display rendering of raw property values from binary trees
//! (.dtb blobs and /proc/device-tree), where no type information exists.

const MAX_CELLS: usize = 256;
const MAX_BYTES: usize = 512;

pub fn render_value(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if let Some(strings) = as_string_list(bytes) {
        return strings
            .iter()
            .map(|s| format!("\"{}\"", s.escape_default()))
            .collect::<Vec<_>>()
            .join(", ");
    }
    if bytes.len().is_multiple_of(4) {
        let total = bytes.len() / 4;
        let shown = total.min(MAX_CELLS);
        let cells: Vec<String> = bytes
            .chunks_exact(4)
            .take(shown)
            .map(|c| format!("0x{:08x}", u32::from_be_bytes([c[0], c[1], c[2], c[3]])))
            .collect();
        let mut out = format!("<{}>", cells.join(" "));
        if shown < total {
            out.push_str(&format!(" /* truncated: {total} cells total */"));
        }
        return out;
    }
    let total = bytes.len();
    let shown = total.min(MAX_BYTES);
    let hex: Vec<String> = bytes
        .iter()
        .take(shown)
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut out = format!("[{}]", hex.join(" "));
    if shown < total {
        out.push_str(&format!(" /* truncated: {total} bytes total */"));
    }
    out
}

/// Treat the value as a NUL-terminated string list if every byte is printable
/// ASCII or NUL, it ends with NUL, and no segment is empty.
fn as_string_list(bytes: &[u8]) -> Option<Vec<String>> {
    if *bytes.last()? != 0 {
        return None;
    }
    if !bytes.iter().all(|&b| b == 0 || (0x20..=0x7e).contains(&b)) {
        return None;
    }
    let mut out = Vec::new();
    for seg in bytes[..bytes.len() - 1].split(|&b| b == 0) {
        if seg.is_empty() {
            return None;
        }
        out.push(String::from_utf8_lossy(seg).into_owned());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_boolean() {
        assert_eq!(render_value(&[]), "");
    }

    #[test]
    fn string_list() {
        assert_eq!(render_value(b"okay\0"), "\"okay\"");
        assert_eq!(
            render_value(b"ns16550a\0snps,dw-apb-uart\0"),
            "\"ns16550a\", \"snps,dw-apb-uart\""
        );
    }

    #[test]
    fn cells() {
        assert_eq!(
            render_value(&[0, 0, 0, 1, 0, 0, 0x10, 0]),
            "<0x00000001 0x00001000>"
        );
    }

    #[test]
    fn odd_bytes() {
        assert_eq!(render_value(&[0xde, 0xad, 0xbe]), "[de ad be]");
    }

    #[test]
    fn four_byte_string_is_still_string() {
        // "ok\0" is 3 bytes -> string; "abc\0" is 4 bytes and printable -> string wins over cells
        assert_eq!(render_value(b"abc\0"), "\"abc\"");
    }
}
