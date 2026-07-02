//! Parses the demo device tree bundled in examples/ end-to-end, keeping the
//! fixtures valid as the parser evolves.

use miru_dt_lib::dts::parse_dts_file;
use miru_dt_lib::model::{DtNode, DtProperty};
use std::path::Path;

fn child<'a>(node: &'a DtNode, name: &str) -> &'a DtNode {
    node.children
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no child {name} under {}", node.name))
}

fn prop<'a>(node: &'a DtNode, name: &str) -> &'a DtProperty {
    node.properties
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no property {name} on {}", node.name))
}

#[test]
fn parses_the_bundled_examples() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples");
    let include_dir = examples.join("include").display().to_string();
    let r = parse_dts_file(&examples.join("board.dts"), &[include_dir]).unwrap();
    assert!(
        r.warnings.is_empty(),
        "unexpected warnings: {:?}",
        r.warnings
    );

    let graph = r.include_graph.as_ref().unwrap();
    assert_eq!(
        graph.edges.len(),
        2,
        "board.dts has two #include directives"
    );
    assert_eq!(graph.files.len(), 3);
    assert!(graph.edges.iter().all(|e| e.directive == "#include"));

    // uart0: defined disabled by the SoC include, enabled by the board.
    let soc = child(&r.tree, "soc");
    let uart = child(soc, "serial@10000000");
    assert_eq!(uart.labels, vec!["uart0"]);
    let status = prop(uart, "status");
    assert_eq!(status.value, "\"okay\"");
    let sp = status.provenance.as_ref().unwrap();
    assert!(sp.defined.file.ends_with("soc.dtsi"));
    assert_eq!(sp.modified.len(), 1);
    assert!(sp.modified[0].file.ends_with("board.dts"));

    // Macro from the dt-bindings header is expanded in the property value.
    let led = child(child(&r.tree, "leds"), "led-status");
    assert_eq!(prop(led, "gpios").value, "<&gpio0 5 0>");

    // The watchdog is deleted by the board but stays visible as deleted.
    let wdt = child(soc, "watchdog@10070000");
    assert!(wdt.deleted);
    assert!(wdt.provenance.as_ref().unwrap().modified[0]
        .file
        .ends_with("board.dts"));

    // Reference-valued property.
    assert_eq!(prop(child(&r.tree, "aliases"), "serial0").value, "&uart0");
}
