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

- **Rust backend** (`src-tauri/src/`): parses the three device tree input formats and normalizes them into a common tree model exposed to the frontend over Tauri IPC commands. For `.dts` input it also resolves `/include/` and `#include` directives to build the include dependency graph, and records source provenance while merging the tree: every node and property in the model carries the file/line where it was defined plus the ordered list of file/line locations that later modified or overrode it. Provenance only exists for source input — trees loaded from `.dtb`/`.dtbo` or `/proc/device-tree` have none.
- **Web frontend** (`src/routes/`): Svelte 5 + TypeScript, rendering the device tree, the include dependency graph, and per-node provenance (definition and modification sites) in the Tauri webview. It calls backend commands via `invoke` from `@tauri-apps/api`.
