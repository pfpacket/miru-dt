//! Reader for the live device tree exposed by the Linux kernel at
//! /proc/device-tree (a symlink to /sys/firmware/devicetree/base):
//! directories are nodes, files are properties.

use crate::model::{DtNode, DtProperty, LoadResult};
use crate::render::render_value;
use std::path::Path;

pub fn load(path: &str) -> Result<LoadResult, String> {
    let p = Path::new(path);
    if !p.is_dir() {
        return Err(format!(
            "{path} is not a readable directory — is this a Linux system with device tree support?"
        ));
    }
    let mut warnings = Vec::new();
    let tree = read_node(p, "/", &mut warnings)?;
    Ok(LoadResult {
        source: path.to_string(),
        kind: "live".into(),
        tree,
        include_graph: None,
        warnings,
    })
}

fn read_node(dir: &Path, name: &str, warnings: &mut Vec<String>) -> Result<DtNode, String> {
    let mut node = DtNode::new(name);
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let entry_name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else {
            warnings.push(format!("{}: cannot stat", entry.path().display()));
            continue;
        };
        if file_type.is_dir() {
            match read_node(&entry.path(), &entry_name, warnings) {
                Ok(child) => node.children.push(child),
                Err(w) => warnings.push(w),
            }
        } else {
            match std::fs::read(entry.path()) {
                Ok(bytes) => node.properties.push(DtProperty {
                    name: entry_name,
                    value: render_value(&bytes),
                    deleted: false,
                    provenance: None,
                }),
                Err(e) => warnings.push(format!("{}: {e}", entry.path().display())),
            }
        }
    }
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_directory_tree() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("compatible"), b"test,machine\0").unwrap();
        fs::create_dir(dir.path().join("cpus")).unwrap();
        fs::create_dir(dir.path().join("cpus").join("cpu@0")).unwrap();
        fs::write(
            dir.path().join("cpus").join("cpu@0").join("reg"),
            [0, 0, 0, 0],
        )
        .unwrap();

        let r = load(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(r.kind, "live");
        assert_eq!(r.tree.name, "/");
        assert_eq!(r.tree.properties[0].name, "compatible");
        assert_eq!(r.tree.properties[0].value, "\"test,machine\"");
        let cpu = &r.tree.children[0].children[0];
        assert_eq!(cpu.name, "cpu@0");
        assert_eq!(cpu.properties[0].value, "<0x00000000>");
        assert!(cpu.provenance.is_none());
    }

    #[test]
    fn missing_dir_errors() {
        assert!(load("/nonexistent/device-tree").is_err());
    }
}
