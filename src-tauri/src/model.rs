use serde::Serialize;

/// A position in a device tree source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceLoc {
    pub file: String,
    pub line: u32,
}

impl SourceLoc {
    pub fn new(file: impl Into<String>, line: u32) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }
}

impl std::fmt::Display for SourceLoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// Where a node or property came from: the site that first defined it and,
/// in source order, every later site that touched it (re-opened it, overrode
/// its value, deleted it, or re-defined it after deletion).
#[derive(Debug, Clone, Serialize)]
pub struct Provenance {
    pub defined: SourceLoc,
    pub modified: Vec<SourceLoc>,
}

impl Provenance {
    pub fn new(defined: SourceLoc) -> Self {
        Self {
            defined,
            modified: Vec::new(),
        }
    }

    pub fn touch(&mut self, loc: SourceLoc) {
        self.modified.push(loc);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DtProperty {
    pub name: String,
    /// Display form of the value: reconstructed source text for `.dts` input,
    /// decoded form (strings / cells / bytes) for binary input. Empty for
    /// boolean (value-less) properties.
    pub value: String,
    /// Removed by /delete-property/ (kept in the model so the deletion site
    /// remains visible).
    pub deleted: bool,
    /// None for trees without source information (.dtb, /proc/device-tree).
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DtNode {
    pub name: String,
    pub labels: Vec<String>,
    pub properties: Vec<DtProperty>,
    pub children: Vec<DtNode>,
    /// Removed by /delete-node/ (kept in the model so the deletion site
    /// remains visible).
    pub deleted: bool,
    /// None for trees without source information (.dtb, /proc/device-tree).
    pub provenance: Option<Provenance>,
}

impl DtNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            labels: Vec::new(),
            properties: Vec::new(),
            children: Vec::new(),
            deleted: false,
            provenance: None,
        }
    }

    pub fn child_mut(&mut self, name: &str) -> Option<&mut DtNode> {
        self.children.iter_mut().find(|c| c.name == name)
    }

    pub fn property_mut(&mut self, name: &str) -> Option<&mut DtProperty> {
        self.properties.iter_mut().find(|p| p.name == name)
    }

    /// Descend from this node along an absolute device tree path like
    /// `/soc/serial@10000000`. `/` returns the node itself.
    pub fn node_at_path_mut(&mut self, path: &str) -> Option<&mut DtNode> {
        let mut cur = self;
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            cur = cur.child_mut(seg)?;
        }
        Some(cur)
    }
}

/// One resolved include directive: `from` pulls in `to` at `from:line`.
#[derive(Debug, Clone, Serialize)]
pub struct IncludeEdge {
    pub from: String,
    pub to: String,
    pub line: u32,
    /// "/include/" or "#include"
    pub directive: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IncludeGraph {
    pub root: String,
    pub files: Vec<String>,
    pub edges: Vec<IncludeEdge>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadResult {
    /// Path of the loaded file or directory.
    pub source: String,
    /// "dts" | "dtb" | "live"
    pub kind: String,
    pub tree: DtNode,
    /// Only present for "dts" input.
    pub include_graph: Option<IncludeGraph>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JSON field names are the IPC contract with src/lib/types.ts.
    #[test]
    fn serialization_matches_frontend_types() {
        let mut node = DtNode::new("/");
        node.provenance = Some(Provenance::new(SourceLoc::new("a.dts", 1)));
        node.properties.push(DtProperty {
            name: "status".into(),
            value: "\"okay\"".into(),
            deleted: false,
            provenance: Some(Provenance::new(SourceLoc::new("a.dts", 2))),
        });
        let result = LoadResult {
            source: "a.dts".into(),
            kind: "dts".into(),
            tree: node,
            include_graph: Some(IncludeGraph {
                root: "a.dts".into(),
                files: vec!["a.dts".into()],
                edges: vec![IncludeEdge {
                    from: "a.dts".into(),
                    to: "b.dtsi".into(),
                    line: 3,
                    directive: "#include".into(),
                }],
            }),
            warnings: vec![],
        };
        let v = serde_json::to_value(&result).unwrap();
        assert!(v.get("includeGraph").is_some(), "camelCase includeGraph");
        assert!(v["tree"]["provenance"]["defined"]["file"].is_string());
        assert!(v["tree"]["provenance"]["modified"].is_array());
        let p = &v["tree"]["properties"][0];
        for key in ["name", "value", "deleted", "provenance"] {
            assert!(p.get(key).is_some(), "property field {key}");
        }
        let e = &v["includeGraph"]["edges"][0];
        for key in ["from", "to", "line", "directive"] {
            assert!(e.get(key).is_some(), "edge field {key}");
        }
    }
}
