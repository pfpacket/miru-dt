//! Phandle resolution for binary trees (.dtb blobs and /proc/device-tree).
//!
//! Compiled trees replace `&label` references with numeric phandles. This
//! module turns a raw (bytes-per-property) tree into the display model,
//! resolving phandles back to the nodes they point at: a phandle map is
//! collected from `phandle`/`linux,phandle` properties (plus labels from
//! `__symbols__` when the blob was compiled with `dtc -@`), and properties
//! known to carry references are decoded to `&label` / `&{/node/path}` form.
//! Phandle+args lists (`clocks`, `gpios`, ...) are walked using the
//! referenced node's `#*-cells` value. If any part of a property fails to
//! resolve, the whole property falls back to the plain heuristic rendering —
//! never a half-guessed decode.

use crate::model::{DtNode, DtProperty};
use crate::render::render_value;
use std::collections::HashMap;

/// Tree shape shared by the DTB parser and the live-tree reader before
/// property values are rendered for display.
pub struct RawNode {
    pub name: String,
    pub properties: Vec<(String, Vec<u8>)>,
    pub children: Vec<RawNode>,
}

enum RefKind {
    /// Every cell is a phandle (`interrupt-parent`, `*-supply`, ...).
    Single,
    /// List of phandle + N argument cells, N given by this `#*-cells`
    /// property on the referenced node.
    WithArgs(&'static str),
}

fn classify(name: &str) -> Option<RefKind> {
    use RefKind::*;
    Some(match name {
        "interrupt-parent" | "phy-handle" | "remote-endpoint" | "next-level-cache" | "l2-cache"
        | "memory-region" | "cpu" | "cpus" => Single,
        "clocks" | "assigned-clocks" | "assigned-clock-parents" => WithArgs("#clock-cells"),
        "resets" => WithArgs("#reset-cells"),
        "dmas" => WithArgs("#dma-cells"),
        "pwms" => WithArgs("#pwm-cells"),
        "phys" => WithArgs("#phy-cells"),
        "mboxes" => WithArgs("#mbox-cells"),
        "io-channels" => WithArgs("#io-channel-cells"),
        "power-domains" => WithArgs("#power-domain-cells"),
        "interrupts-extended" => WithArgs("#interrupt-cells"),
        "sound-dai" => WithArgs("#sound-dai-cells"),
        "thermal-sensors" => WithArgs("#thermal-sensor-cells"),
        "iommus" => WithArgs("#iommu-cells"),
        "interconnects" => WithArgs("#interconnect-cells"),
        "hwlocks" => WithArgs("#hwlock-cells"),
        "cooling-device" => WithArgs("#cooling-cells"),
        "gpios" => WithArgs("#gpio-cells"),
        _ if name.ends_with("-supply") => Single,
        _ if name.ends_with("-gpios") || name.ends_with("-gpio") => WithArgs("#gpio-cells"),
        _ => return None,
    })
}

#[derive(Default)]
struct RefMaps {
    path_by_phandle: HashMap<u32, String>,
    label_by_path: HashMap<String, String>,
    cells_by_phandle: HashMap<u32, HashMap<String, u32>>,
}

fn be32(bytes: &[u8]) -> Option<u32> {
    (bytes.len() == 4).then(|| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn cells_of(bytes: &[u8]) -> Option<Vec<u32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

fn cstr(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|&b| b == 0)?;
    std::str::from_utf8(&bytes[..end]).ok().map(str::to_string)
}

pub fn into_model(root: &RawNode) -> DtNode {
    let mut maps = RefMaps::default();
    collect(root, "/", &mut maps);
    build(root, &maps)
}

fn collect(node: &RawNode, path: &str, maps: &mut RefMaps) {
    if node.name == "__symbols__" {
        for (label, bytes) in &node.properties {
            if let Some(target) = cstr(bytes) {
                maps.label_by_path.insert(target, label.clone());
            }
        }
        return;
    }
    let phandle = node
        .properties
        .iter()
        .find(|(n, _)| n == "phandle" || n == "linux,phandle")
        .and_then(|(_, b)| be32(b));
    if let Some(ph) = phandle {
        maps.path_by_phandle.insert(ph, path.to_string());
        let cells: HashMap<String, u32> = node
            .properties
            .iter()
            .filter(|(n, _)| n.starts_with('#') && n.ends_with("-cells"))
            .filter_map(|(n, b)| be32(b).map(|v| (n.clone(), v)))
            .collect();
        maps.cells_by_phandle.insert(ph, cells);
    }
    for child in &node.children {
        let child_path = if path == "/" {
            format!("/{}", child.name)
        } else {
            format!("{path}/{}", child.name)
        };
        collect(child, &child_path, maps);
    }
}

fn build(node: &RawNode, maps: &RefMaps) -> DtNode {
    let mut out = DtNode::new(node.name.clone());
    for (name, bytes) in &node.properties {
        out.properties.push(DtProperty {
            name: name.clone(),
            value: decode(name, bytes, maps),
            deleted: false,
            provenance: None,
        });
    }
    for child in &node.children {
        out.children.push(build(child, maps));
    }
    out
}

fn ref_str(path: &str, maps: &RefMaps) -> String {
    match maps.label_by_path.get(path) {
        Some(label) => format!("&{label}"),
        None => format!("&{{{path}}}"),
    }
}

fn decode(name: &str, bytes: &[u8], maps: &RefMaps) -> String {
    if let Some(kind) = classify(name) {
        if let Some(cells) = cells_of(bytes) {
            if let Some(decoded) = decode_refs(&kind, &cells, maps) {
                return decoded;
            }
        }
    }
    render_value(bytes)
}

fn decode_refs(kind: &RefKind, cells: &[u32], maps: &RefMaps) -> Option<String> {
    let mut entries: Vec<String> = Vec::new();
    match kind {
        RefKind::Single => {
            for &ph in cells {
                let path = maps.path_by_phandle.get(&ph)?;
                entries.push(format!("<{}>", ref_str(path, maps)));
            }
        }
        RefKind::WithArgs(cells_prop) => {
            let mut i = 0usize;
            while i < cells.len() {
                let path = maps.path_by_phandle.get(&cells[i])?;
                let argc = *maps.cells_by_phandle.get(&cells[i])?.get(*cells_prop)? as usize;
                if i + 1 + argc > cells.len() {
                    return None;
                }
                let mut entry = ref_str(path, maps);
                for arg in &cells[i + 1..i + 1 + argc] {
                    entry.push_str(&format!(" 0x{arg:x}"));
                }
                entries.push(format!("<{entry}>"));
                i += 1 + argc;
            }
        }
    }
    Some(entries.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32be(v: u32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    fn demo_tree() -> RawNode {
        RawNode {
            name: "/".into(),
            properties: vec![],
            children: vec![
                RawNode {
                    name: "clk".into(),
                    properties: vec![
                        ("phandle".into(), u32be(5)),
                        ("#clock-cells".into(), u32be(1)),
                    ],
                    children: vec![],
                },
                RawNode {
                    name: "sram".into(),
                    properties: vec![("phandle".into(), u32be(6))],
                    children: vec![],
                },
                RawNode {
                    name: "uart".into(),
                    properties: vec![
                        ("clocks".into(), [u32be(5), u32be(3)].concat()),
                        ("memory-region".into(), u32be(6)),
                        ("reg".into(), u32be(0x1000)),
                    ],
                    children: vec![],
                },
            ],
        }
    }

    fn prop_value(node: &DtNode, child: &str, name: &str) -> String {
        node.children
            .iter()
            .find(|c| c.name == child)
            .unwrap()
            .properties
            .iter()
            .find(|p| p.name == name)
            .unwrap()
            .value
            .clone()
    }

    #[test]
    fn decodes_phandle_with_args_and_single() {
        let tree = into_model(&demo_tree());
        assert_eq!(prop_value(&tree, "uart", "clocks"), "<&{/clk} 0x3>");
        assert_eq!(prop_value(&tree, "uart", "memory-region"), "<&{/sram}>");
        // Non-reference properties keep the plain rendering.
        assert_eq!(prop_value(&tree, "uart", "reg"), "<0x00001000>");
    }

    #[test]
    fn uses_symbols_labels_when_present() {
        let mut raw = demo_tree();
        raw.children.push(RawNode {
            name: "__symbols__".into(),
            properties: vec![("clk0".into(), b"/clk\0".to_vec())],
            children: vec![],
        });
        let tree = into_model(&raw);
        assert_eq!(prop_value(&tree, "uart", "clocks"), "<&clk0 0x3>");
    }

    #[test]
    fn unknown_phandle_falls_back_to_raw() {
        let mut raw = demo_tree();
        raw.children[2].properties[0] = ("clocks".into(), [u32be(99), u32be(3)].concat());
        let tree = into_model(&raw);
        assert_eq!(
            prop_value(&tree, "uart", "clocks"),
            "<0x00000063 0x00000003>"
        );
    }

    #[test]
    fn missing_cells_property_falls_back_to_raw() {
        let mut raw = demo_tree();
        raw.children[0]
            .properties
            .retain(|(n, _)| n != "#clock-cells");
        let tree = into_model(&raw);
        assert_eq!(
            prop_value(&tree, "uart", "clocks"),
            "<0x00000005 0x00000003>"
        );
    }

    #[test]
    fn truncated_args_fall_back_to_raw() {
        let mut raw = demo_tree();
        // phandle expects one arg cell, but the list ends after the phandle
        raw.children[2].properties[0] = ("clocks".into(), u32be(5));
        let tree = into_model(&raw);
        assert_eq!(prop_value(&tree, "uart", "clocks"), "<0x00000005>");
    }
}
