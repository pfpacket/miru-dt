//! Flattened device tree (.dtb / .dtbo) parsing into the common model.
//! Binary trees carry no source information, so nodes and properties have
//! no provenance; property values are rendered heuristically.

use crate::model::{DtNode, DtProperty, LoadResult};
use crate::render::render_value;
use std::path::Path;

const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

fn be32(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("truncated blob at offset {off}"))
}

fn cstr(b: &[u8], off: usize) -> Result<(String, usize), String> {
    let rel = b
        .get(off..)
        .and_then(|s| s.iter().position(|&c| c == 0))
        .ok_or_else(|| format!("unterminated string at offset {off}"))?;
    Ok((
        String::from_utf8_lossy(&b[off..off + rel]).into_owned(),
        rel,
    ))
}

pub fn parse_dtb(bytes: &[u8]) -> Result<DtNode, String> {
    let magic = be32(bytes, 0)?;
    if magic != FDT_MAGIC {
        return Err(format!("not a device tree blob (bad magic 0x{magic:08x})"));
    }
    let off_struct = be32(bytes, 8)? as usize;
    let off_strings = be32(bytes, 12)? as usize;

    let mut pos = off_struct;
    let mut stack: Vec<DtNode> = Vec::new();
    loop {
        let token = be32(bytes, pos)?;
        pos += 4;
        match token {
            FDT_BEGIN_NODE => {
                let (name, len) = cstr(bytes, pos)?;
                pos = (pos + len + 1 + 3) & !3;
                let name = if name.is_empty() {
                    "/".to_string()
                } else {
                    name
                };
                stack.push(DtNode::new(name));
            }
            FDT_END_NODE => {
                let node = stack.pop().ok_or("unbalanced FDT_END_NODE")?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return Ok(node),
                }
            }
            FDT_PROP => {
                let len = be32(bytes, pos)? as usize;
                let nameoff = be32(bytes, pos + 4)? as usize;
                pos += 8;
                let value = bytes
                    .get(pos..pos + len)
                    .ok_or_else(|| format!("truncated property value at offset {pos}"))?;
                pos = (pos + len + 3) & !3;
                let (name, _) = cstr(bytes, off_strings + nameoff)?;
                let node = stack.last_mut().ok_or("property outside of any node")?;
                node.properties.push(DtProperty {
                    name,
                    value: render_value(value),
                    deleted: false,
                    provenance: None,
                });
            }
            FDT_NOP => {}
            FDT_END => return Err("blob ends without a root node".into()),
            other => {
                return Err(format!(
                    "unknown FDT token 0x{other:08x} at offset {}",
                    pos - 4
                ))
            }
        }
    }
}

pub fn load(path: &Path) -> Result<LoadResult, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let tree = parse_dtb(&bytes)?;
    Ok(LoadResult {
        source: path.display().to_string(),
        kind: "dtb".into(),
        tree,
        include_graph: None,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Builder {
        struct_block: Vec<u8>,
        strings: Vec<u8>,
    }

    impl Builder {
        fn new() -> Self {
            Self {
                struct_block: Vec::new(),
                strings: Vec::new(),
            }
        }

        fn u32(&mut self, v: u32) {
            self.struct_block.extend_from_slice(&v.to_be_bytes());
        }

        fn begin_node(&mut self, name: &str) {
            self.u32(FDT_BEGIN_NODE);
            self.struct_block.extend_from_slice(name.as_bytes());
            self.struct_block.push(0);
            while !self.struct_block.len().is_multiple_of(4) {
                self.struct_block.push(0);
            }
        }

        fn end_node(&mut self) {
            self.u32(FDT_END_NODE);
        }

        fn prop(&mut self, name: &str, value: &[u8]) {
            let nameoff = self.strings.len() as u32;
            self.strings.extend_from_slice(name.as_bytes());
            self.strings.push(0);
            self.u32(FDT_PROP);
            self.u32(value.len() as u32);
            self.u32(nameoff);
            self.struct_block.extend_from_slice(value);
            while !self.struct_block.len().is_multiple_of(4) {
                self.struct_block.push(0);
            }
        }

        fn finish(mut self) -> Vec<u8> {
            self.u32(FDT_END);
            let header_len = 40usize;
            let off_struct = header_len;
            let off_strings = off_struct + self.struct_block.len();
            let total = off_strings + self.strings.len();
            let mut out = Vec::new();
            for v in [
                FDT_MAGIC,
                total as u32,
                off_struct as u32,
                off_strings as u32,
                0, // off_mem_rsvmap (unused by the parser)
                17,
                16,
                0,
                self.strings.len() as u32,
                self.struct_block.len() as u32,
            ] {
                out.extend_from_slice(&v.to_be_bytes());
            }
            out.extend_from_slice(&self.struct_block);
            out.extend_from_slice(&self.strings);
            out
        }
    }

    #[test]
    fn parses_minimal_blob() {
        let mut b = Builder::new();
        b.begin_node("");
        b.prop("compatible", b"test,root\0");
        b.begin_node("uart@10000000");
        b.prop("status", b"okay\0");
        b.prop("reg", &[0x10, 0x00, 0x00, 0x00]);
        b.end_node();
        b.end_node();
        let blob = b.finish();

        let root = parse_dtb(&blob).unwrap();
        assert_eq!(root.name, "/");
        assert_eq!(root.properties[0].name, "compatible");
        assert_eq!(root.properties[0].value, "\"test,root\"");
        assert!(root.properties[0].provenance.is_none());
        let uart = &root.children[0];
        assert_eq!(uart.name, "uart@10000000");
        assert_eq!(uart.properties[0].value, "\"okay\"");
        assert_eq!(uart.properties[1].value, "<0x10000000>");
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(parse_dtb(&[0u8; 64]).unwrap_err().contains("bad magic"));
    }
}
