# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

miru-dt is a device tree visualizer: a Tauri-based GUI app for inspecting Linux device trees. It will support three input sources:

- `.dts` source files
- compiled `.dtb` / `.dtbo` blobs
- the live device tree at `/proc/device-tree` on a running Linux system

Beyond the tree structure itself, it must also visualize:

- The include dependency graph of device tree source files — which `.dts`/`.dtsi` files pull in which others via `/include/` and C-preprocessor `#include` directives.
- Per-node provenance — which file defines each node, which files subsequently modify it (e.g. re-opened nodes or label references like `&label { ... }` overriding properties across the include chain), and the exact location (file and line) of each definition and modification.

## Commands

JS tooling uses bun. Rust follows the standard cargo workflow (rustfmt and clippy with default lints); cargo commands run inside `src-tauri/`.

```sh
bun install             # install frontend dependencies
bun run tauri dev       # run the app with hot reload
bun run tauri build     # production build + bundle
bun run check           # svelte-check (frontend typecheck)

cd src-tauri
cargo check
cargo test              # run all Rust tests
cargo test <test_name>  # run a single Rust test
cargo fmt
cargo clippy
```

## Architecture

Tauri 2 app: SvelteKit frontend (static adapter, SSR disabled) in the repo root, Rust backend crate in `src-tauri/`. The Vite dev server runs on port 1420; `tauri dev`/`tauri build` invoke the frontend build via `beforeDevCommand`/`beforeBuildCommand` in `src-tauri/tauri.conf.json`.

- **Rust backend** (`src-tauri/src/`): parses the three device tree input formats and normalizes them into a common tree model (`model.rs`) exposed over the Tauri IPC commands `load_dts`, `load_dtb`, `load_live` (`lib.rs`).
  - `dts.rs` — the core: lexer that inlines `/include/`/`#include` files into one (file, line)-tagged token stream while recording include edges, a C-preprocessor subset (`#define` object/function macros, `#if(def)` conditionals, include guards), and a parser that merges all top-level blocks (`/ { }` re-opens, `&label { }` overrides, `/delete-node/`, `/delete-property/`) into a single tree. Provenance is recorded during that merge: every node/property carries its definition file/line plus the ordered list of later modification sites. Deleted nodes/properties stay in the model flagged `deleted` so the deletion site remains visible. Property values are reconstructed source text (macros expanded), not compiled cells — do not shell out to `dtc`, which would discard provenance.
  - `dtb.rs` / `live.rs` — flattened blob parser and `/proc/device-tree` directory reader; both produce trees without provenance, with property bytes rendered heuristically (`render.rs`: strings / u32 cells / byte list).
  - Parser behavior is covered by unit tests in each module, plus `tests/examples.rs` (parses `examples/board.dts`) and `tests/dtc_roundtrip.rs` (cross-checks the blob parser against `dtc` output; skips if `dtc` is absent).
- **Web frontend** (`src/routes/`, components in `src/lib/`): Svelte 5 + TypeScript. `TreeNode.svelte` (recursive tree with filter), `IncludeGraph.svelte` (indented include hierarchy), and the details panel in `+page.svelte` showing per-node/per-property provenance. `src/lib/types.ts` mirrors `model.rs` — keep them in sync (Rust `include_graph` serializes as camelCase `includeGraph`). File picking uses `@tauri-apps/plugin-dialog`.

`examples/board.dts` (+ `soc.dtsi`, `include/dt-bindings/...`) is a demo tree: load it with include dir `examples/include` to exercise the include graph, provenance, macro expansion, and delete handling.

## Environment Notes

When running the app over SSH X-forwarding, WebKitGTK crashes with an X11 `BadRequest` error unless compositing is disabled:

```sh
WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 bun run tauri dev
```
