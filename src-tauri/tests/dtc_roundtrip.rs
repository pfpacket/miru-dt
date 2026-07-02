//! Cross-checks the FDT parser against blobs produced by the reference
//! device tree compiler. Skips silently when `dtc` is not installed.

use miru_dt_lib::dtb::parse_dtb;
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
