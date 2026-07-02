# miru-dt

A device tree visualizer: a Tauri GUI app for inspecting Linux device trees.

## Features

- **Three input sources**: `.dts`/`.dtsi` source files, compiled `.dtb`/`.dtbo` blobs, and the
  live tree at `/proc/device-tree` on a running Linux system.
- **Include dependency graph**: see which `.dts`/`.dtsi`/header files pull in which others via
  `/include/` and `#include`, with the line of every directive.
- **Per-node provenance** (source input): for every node and property, the file/line where it
  was defined and every later site that modified it — re-opened nodes, `&label { … }` overrides,
  property overrides, `/delete-node/` and `/delete-property/` (deleted items stay visible,
  struck through, so the deletion site remains traceable).
- **Phandle resolution** (blob and live input): numeric phandles in reference properties
  (`interrupt-parent`, `clocks`, `*-gpios`, `*-supply`, …) are resolved back to the node they
  point at and shown as `&label` (when the blob has `__symbols__`, i.e. compiled with
  `dtc -@`) or `&{/node/path}`; phandle+args lists are decoded using the target's `#*-cells`.
- Filterable tree, macro expansion from `dt-bindings` headers, warnings panel for unresolved
  references and preprocessor issues.

## Running

```sh
bun install
bun run tauri dev
```

Over SSH X-forwarding, disable WebKit compositing first:

```sh
WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 bun run tauri dev
```

## Try the demo

Open `examples/board.dts` with include directory `examples/include` (set it in the toolbar
before opening). The board file includes `soc.dtsi` and a `dt-bindings` header; the SoC defines
devices as `disabled` and the board enables them, so the provenance panel shows
defined-in-soc / modified-in-board for the interesting nodes.

For real kernel trees, typical include directories are `<kernel>/include` and
`<kernel>/scripts/dtc/include-prefixes`.

## Development

```sh
bun run check           # frontend typecheck (svelte-check)
cd src-tauri
cargo test              # parser unit + integration tests
cargo clippy
cargo fmt
```
