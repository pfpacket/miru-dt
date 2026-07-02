//! Cross-checks the FDT parser against blobs produced by the reference
//! device tree compiler. Skips silently when `dtc` is not installed.

use miru_dt_lib::dtb::parse_dtb;
use miru_dt_lib::model::DtNode;
use std::process::Command;

#[test]
fn parses_dtc_compiled_blob() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("t.dts");
    let blob = dir.path().join("t.dtb");
    std::fs::write(
        &src,
        concat!(
            "/dts-v1/;\n",
            "/ {\n",
            "    compatible = \"test,board\", \"test,soc\";\n",
            "    #address-cells = <1>;\n",
            "    soc {\n",
            "        uart@10000000 {\n",
            "            reg = <0x10000000 0x100>;\n",
            "            wakeup-source;\n",
            "            mac = [00 11 22];\n",
            "            status = \"okay\";\n",
            "        };\n",
            "    };\n",
            "};\n",
        ),
    )
    .unwrap();

    let dtc = Command::new("dtc")
        .args(["-I", "dts", "-O", "dtb", "-o"])
        .arg(&blob)
        .arg(&src)
        .output();
    let Ok(out) = dtc else {
        eprintln!("dtc not installed; skipping round-trip test");
        return;
    };
    assert!(
        out.status.success(),
        "dtc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let root = parse_dtb(&std::fs::read(&blob).unwrap()).unwrap();
    assert_eq!(root.name, "/");
    let compat = root
        .properties
        .iter()
        .find(|p| p.name == "compatible")
        .unwrap();
    assert_eq!(compat.value, "\"test,board\", \"test,soc\"");
    let cells = root
        .properties
        .iter()
        .find(|p| p.name == "#address-cells")
        .unwrap();
    assert_eq!(cells.value, "<0x00000001>");

    let uart = &root
        .children
        .iter()
        .find(|c| c.name == "soc")
        .unwrap()
        .children[0];
    assert_eq!(uart.name, "uart@10000000");
    let by_name = |n: &str| {
        uart.properties
            .iter()
            .find(|p| p.name == n)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(by_name("reg"), "<0x10000000 0x00000100>");
    assert_eq!(by_name("wakeup-source"), "");
    assert_eq!(by_name("mac"), "[00 11 22]");
    assert_eq!(by_name("status"), "\"okay\"");
}

const REF_DTS: &str = concat!(
    "/dts-v1/;\n",
    "/ {\n",
    "    intc: intc {\n",
    "        interrupt-controller;\n",
    "        #interrupt-cells = <2>;\n",
    "    };\n",
    "    clk: clk {\n",
    "        #clock-cells = <1>;\n",
    "    };\n",
    "    vcc: regulator-vcc {\n",
    "    };\n",
    "    uart {\n",
    "        interrupt-parent = <&intc>;\n",
    "        interrupts = <5 4>;\n",
    "        clocks = <&clk 3>, <&clk 4>;\n",
    "        vcc-supply = <&vcc>;\n",
    "    };\n",
    "};\n",
);

fn compile_and_parse(extra_args: &[&str]) -> Option<DtNode> {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("t.dts");
    let blob = dir.path().join("t.dtb");
    std::fs::write(&src, REF_DTS).unwrap();
    let mut args: Vec<&str> = vec!["-I", "dts", "-O", "dtb"];
    args.extend_from_slice(extra_args);
    let out = Command::new("dtc")
        .args(&args)
        .arg("-o")
        .arg(&blob)
        .arg(&src)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_dtb(&std::fs::read(&blob).unwrap()).unwrap())
}

fn uart_prop(root: &DtNode, name: &str) -> String {
    root.children
        .iter()
        .find(|c| c.name == "uart")
        .unwrap()
        .properties
        .iter()
        .find(|p| p.name == name)
        .unwrap()
        .value
        .clone()
}

#[test]
fn resolves_phandles_to_node_paths() {
    let Some(root) = compile_and_parse(&[]) else {
        eprintln!("dtc not installed; skipping");
        return;
    };
    assert_eq!(uart_prop(&root, "interrupt-parent"), "<&{/intc}>");
    assert_eq!(uart_prop(&root, "clocks"), "<&{/clk} 0x3>, <&{/clk} 0x4>");
    assert_eq!(uart_prop(&root, "vcc-supply"), "<&{/regulator-vcc}>");
    // Non-reference cells are untouched.
    assert_eq!(uart_prop(&root, "interrupts"), "<0x00000005 0x00000004>");
}

#[test]
fn resolves_phandles_to_labels_with_symbols() {
    // dtc -@ emits a __symbols__ node mapping labels to paths.
    let Some(root) = compile_and_parse(&["-@"]) else {
        eprintln!("dtc not installed or lacks -@; skipping");
        return;
    };
    assert_eq!(uart_prop(&root, "interrupt-parent"), "<&intc>");
    assert_eq!(uart_prop(&root, "clocks"), "<&clk 0x3>, <&clk 0x4>");
    assert_eq!(uart_prop(&root, "vcc-supply"), "<&vcc>");
}
