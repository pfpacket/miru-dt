<script lang="ts">
  import { onMount } from 'svelte';
  import type { DtNode, LoadResult, SourceLoc } from '$lib/types';
  import { loadDtb, loadDts, loadLive, pickDtbFile, pickDtsFile } from '$lib/api';
  import TreeNode from '$lib/TreeNode.svelte';
  import IncludeGraph from '$lib/IncludeGraph.svelte';
  import DependencyGraph from '$lib/DependencyGraph.svelte';

  type LastLoad =
    | { kind: 'dts'; path: string }
    | { kind: 'dtb'; path: string }
    | { kind: 'live' };

  let result = $state<LoadResult | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let filter = $state('');
  let includeDirsRaw = $state('');
  let selectedPath = $state<string | null>(null);
  let selectedNode = $state<DtNode | null>(null);
  let tab = $state<'details' | 'includes' | 'graph' | 'warnings'>('details');
  let lastLoad = $state<LastLoad | null>(null);

  const includeDirs = $derived(
    includeDirsRaw
      .split(':')
      .map((d) => d.trim())
      .filter((d) => d.length > 0)
  );

  const sourceDir = $derived(
    result && result.kind !== 'live' ? result.source.replace(/\/[^/]*$/, '') : ''
  );

  function shorten(file: string): string {
    return sourceDir && file.startsWith(sourceDir + '/') ? file.slice(sourceDir.length + 1) : file;
  }

  function fmtLoc(loc: SourceLoc): string {
    return `${shorten(loc.file)}:${loc.line}`;
  }

  async function run(load: LastLoad) {
    busy = true;
    error = null;
    try {
      if (load.kind === 'dts') {
        result = await loadDts(load.path, includeDirs);
      } else if (load.kind === 'dtb') {
        result = await loadDtb(load.path);
      } else {
        result = await loadLive();
      }
      lastLoad = load;
      selectedPath = '/';
      selectedNode = result.tree;
      tab = 'details';
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function openDts() {
    const path = await pickDtsFile();
    if (path !== null) await run({ kind: 'dts', path });
  }

  async function openDtb() {
    const path = await pickDtbFile();
    if (path !== null) await run({ kind: 'dtb', path });
  }

  function onselect(path: string, node: DtNode) {
    selectedPath = path;
    selectedNode = node;
    tab = 'details';
  }

  // Dev-only autoload for automated UI verification:
  //   VITE_AUTOLOAD=<file> VITE_AUTOLOAD_KIND=<dts|dtb|live> VITE_AUTOLOAD_INC=<dirs>
  //   VITE_AUTOLOAD_SELECT=</node/path> VITE_AUTOLOAD_TAB=<tab> bun run tauri dev
  onMount(() => {
    const auto = import.meta.env.VITE_AUTOLOAD as string | undefined;
    if (!import.meta.env.DEV || typeof auto !== 'string' || auto === '') return;
    includeDirsRaw = (import.meta.env.VITE_AUTOLOAD_INC as string | undefined) ?? '';
    const kind = (import.meta.env.VITE_AUTOLOAD_KIND as string | undefined) ?? 'dts';
    const load: LastLoad =
      kind === 'dtb'
        ? { kind: 'dtb', path: auto }
        : kind === 'live'
          ? { kind: 'live' }
          : { kind: 'dts', path: auto };
    void run(load).then(() => {
      const sel = import.meta.env.VITE_AUTOLOAD_SELECT as string | undefined;
      if (!sel || result === null) return;
      let node: DtNode | undefined = result.tree;
      for (const seg of sel.split('/').filter(Boolean)) {
        node = node?.children.find((c) => c.name === seg);
      }
      if (node !== undefined) onselect(sel, node);
    }).then(() => {
      const t = import.meta.env.VITE_AUTOLOAD_TAB as string | undefined;
      if (t === 'details' || t === 'includes' || t === 'graph' || t === 'warnings') tab = t;
    });
  });
</script>

<div class="app">
  <header>
    <h1>miru-dt</h1>
    <span class="tagline">device tree visualizer</span>
    <div class="actions">
      <button onclick={openDts} disabled={busy}>Open .dts…</button>
      <button onclick={openDtb} disabled={busy}>Open .dtb…</button>
      <button onclick={() => run({ kind: 'live' })} disabled={busy}>/proc/device-tree</button>
      {#if lastLoad !== null}
        <button onclick={() => lastLoad && run(lastLoad)} disabled={busy}>Reload</button>
      {/if}
    </div>
    <input
      class="include-dirs"
      type="text"
      placeholder="include dirs for #include <…> (colon-separated)"
      bind:value={includeDirsRaw}
    />
  </header>

  {#if error !== null}
    <div class="banner error">{error}</div>
  {/if}

  {#if result !== null}
    <div class="source-line">
      <span class="kind-badge {result.kind}">{result.kind}</span>
      <span class="source-path" title={result.source}>{result.source}</span>
      {#if busy}<span class="busy">loading…</span>{/if}
    </div>
    <main>
      <section class="tree-pane">
        <input class="search" type="search" placeholder="Filter nodes, labels, properties…" bind:value={filter} />
        <div class="tree-scroll">
          <TreeNode node={result.tree} path="/" depth={0} {selectedPath} {filter} {onselect} />
        </div>
      </section>

      <section class="side-pane">
        <nav class="tabs">
          <button class:active={tab === 'details'} onclick={() => (tab = 'details')}>Node details</button>
          <button class:active={tab === 'includes'} onclick={() => (tab = 'includes')}>
            Includes{result.includeGraph ? ` (${result.includeGraph.edges.length})` : ''}
          </button>
          <button class:active={tab === 'graph'} onclick={() => (tab = 'graph')}>Graph</button>
          <button class:active={tab === 'warnings'} onclick={() => (tab = 'warnings')}>
            Warnings ({result.warnings.length})
          </button>
        </nav>

        <div class="tab-body">
          {#if tab === 'details'}
            {#if selectedNode !== null}
              <h2 class="node-path">{selectedPath}</h2>
              {#if selectedNode.labels.length > 0}
                <div class="chips">
                  {#each selectedNode.labels as label (label)}
                    <span class="chip">{label}</span>
                  {/each}
                </div>
              {/if}
              {#if selectedNode.deleted}
                <div class="banner deleted-banner">
                  This node was removed with /delete-node/ — shown for provenance.
                </div>
              {/if}

              {#if selectedNode.provenance !== null}
                <div class="prov-block">
                  <div class="prov-row">
                    <span class="prov-kind defined">defined</span>
                    <span class="loc" title={selectedNode.provenance.defined.file}>
                      {fmtLoc(selectedNode.provenance.defined)}
                    </span>
                  </div>
                  {#each selectedNode.provenance.modified as loc, i (i)}
                    <div class="prov-row">
                      <span class="prov-kind modified">modified</span>
                      <span class="loc" title={loc.file}>{fmtLoc(loc)}</span>
                    </div>
                  {/each}
                </div>
              {:else}
                <p class="hint">
                  No source provenance — compiled blobs and the live tree carry no file/line
                  information.
                </p>
              {/if}

              <h3>Properties ({selectedNode.properties.length})</h3>
              {#if selectedNode.properties.length === 0}
                <p class="hint">No properties.</p>
              {/if}
              {#each selectedNode.properties as p, i (i)}
                <div class="prop" class:deleted={p.deleted}>
                  <div class="prop-line">
                    <span class="prop-name">{p.name}</span>
                    {#if p.value !== ''}
                      <span class="prop-eq">=</span>
                      <span class="prop-value">{p.value}</span>
                    {/if}
                    {#if p.provenance !== null && p.provenance.modified.length > 0}
                      <span
                        class="badge modified"
                        title="overridden {p.provenance.modified.length} time{p.provenance.modified.length > 1 ? 's' : ''}"
                      >
                        ✎{p.provenance.modified.length}
                      </span>
                    {/if}
                    {#if p.deleted}<span class="badge removed">deleted</span>{/if}
                  </div>
                  {#if p.provenance !== null}
                    <div class="prop-sites">
                      <div class="prov-row">
                        <span class="prov-kind defined">defined</span>
                        <span class="loc" title={p.provenance.defined.file}>{fmtLoc(p.provenance.defined)}</span>
                      </div>
                      {#each p.provenance.modified as loc, j (j)}
                        <div class="prov-row">
                          <span class="prov-kind modified">modified</span>
                          <span class="loc" title={loc.file}>{fmtLoc(loc)}</span>
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
              {/each}

              <h3>Children ({selectedNode.children.length})</h3>
              {#if selectedNode.children.length > 0}
                <p class="hint">
                  {selectedNode.children.map((c) => c.name).join(', ')}
                </p>
              {:else}
                <p class="hint">Leaf node.</p>
              {/if}
            {:else}
              <p class="hint">Select a node in the tree.</p>
            {/if}
          {:else if tab === 'includes'}
            {#if result.includeGraph !== null}
              <IncludeGraph graph={result.includeGraph} {shorten} />
            {:else}
              <p class="hint">
                Include information only exists for .dts source input — blobs and the live tree
                are already flattened.
              </p>
            {/if}
          {:else if tab === 'graph'}
            {#if result.includeGraph !== null}
              <DependencyGraph graph={result.includeGraph} {shorten} />
            {:else}
              <p class="hint">
                The dependency graph only exists for .dts source input — blobs and the live tree
                are already flattened.
              </p>
            {/if}
          {:else if tab === 'warnings'}
            {#if result.warnings.length === 0}
              <p class="hint">No warnings.</p>
            {:else}
              <ol class="warnings">
                {#each result.warnings as w, i (i)}
                  <li>{w}</li>
                {/each}
              </ol>
            {/if}
          {/if}
        </div>
      </section>
    </main>
  {:else}
    <div class="empty">
      <p>Open a device tree to get started:</p>
      <ul>
        <li><strong>Open .dts…</strong> — parse source with includes, provenance and the include graph</li>
        <li><strong>Open .dtb…</strong> — decode a compiled blob or overlay</li>
        <li><strong>/proc/device-tree</strong> — read the live tree of this machine</li>
      </ul>
      <p class="hint">
        For source with <code>#include &lt;dt-bindings/…&gt;</code>, set the include directories
        first (e.g. <code>path/to/kernel/include:path/to/kernel/scripts/dtc/include-prefixes</code>).
        A demo lives in <code>examples/board.dts</code> with include dir <code>examples/include</code>.
      </p>
      {#if busy}<p class="busy">loading…</p>{/if}
    </div>
  {/if}
</div>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    height: 100%;
  }
  :global(body) {
    background: #0f1115;
    color: #d6dae3;
    font-family:
      Inter,
      system-ui,
      -apple-system,
      sans-serif;
    font-size: 14px;
    --mono: ui-monospace, 'JetBrains Mono', 'Fira Code', Menlo, Consolas, monospace;
    --muted: #7a8394;
    --border: #262c3a;
    --panel: #151924;
    --hover: #1d2330;
    --selected: #24304a;
    --accent: #7aa2ff;
    --accent2: #4fd1c5;
  }
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 14px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    flex-wrap: wrap;
  }
  h1 {
    font-size: 16px;
    margin: 0;
    letter-spacing: 0.5px;
  }
  .tagline {
    color: var(--muted);
    font-size: 12px;
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  button {
    background: #1c2230;
    color: #d6dae3;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 5px 12px;
    font-size: 12.5px;
    cursor: pointer;
  }
  button:hover:enabled {
    border-color: var(--accent);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .include-dirs {
    flex: 1;
    min-width: 240px;
    background: #0f131c;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: #d6dae3;
    padding: 5px 10px;
    font-family: var(--mono);
    font-size: 12px;
  }
  .banner {
    padding: 8px 14px;
    font-size: 13px;
  }
  .banner.error {
    background: #3a1a1a;
    color: #fca5a5;
    font-family: var(--mono);
    white-space: pre-wrap;
  }
  .source-line {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  .kind-badge {
    text-transform: uppercase;
    font-size: 10px;
    font-weight: 700;
    border-radius: 4px;
    padding: 2px 7px;
    letter-spacing: 1px;
  }
  .kind-badge.dts {
    background: #1c3a2a;
    color: #6ee7a8;
  }
  .kind-badge.dtb {
    background: #2a2440;
    color: #c4b5fd;
  }
  .kind-badge.live {
    background: #3a2f14;
    color: #fbbf24;
  }
  .source-path {
    font-family: var(--mono);
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .busy {
    color: var(--accent);
  }
  main {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .tree-pane {
    width: 46%;
    min-width: 280px;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border);
  }
  .search {
    margin: 10px;
    background: #0f131c;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: #d6dae3;
    padding: 6px 10px;
    font-size: 12.5px;
  }
  .tree-scroll {
    overflow: auto;
    flex: 1;
    padding: 0 8px 12px;
  }
  .side-pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 8px 10px 0;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    border: none;
    background: none;
    color: var(--muted);
    border-bottom: 2px solid transparent;
    border-radius: 0;
    padding: 6px 12px;
  }
  .tabs button.active {
    color: #d6dae3;
    border-bottom-color: var(--accent);
  }
  .tab-body {
    overflow: auto;
    padding: 14px 16px;
    flex: 1;
  }
  .node-path {
    font-family: var(--mono);
    font-size: 14px;
    margin: 0 0 8px;
    word-break: break-all;
  }
  .chips {
    display: flex;
    gap: 6px;
    margin-bottom: 8px;
    flex-wrap: wrap;
  }
  .chip {
    background: #10312e;
    color: var(--accent2);
    font-family: var(--mono);
    font-size: 11.5px;
    border-radius: 10px;
    padding: 2px 9px;
  }
  .deleted-banner {
    background: #3a1a1a;
    color: #fca5a5;
    border-radius: 6px;
    margin-bottom: 10px;
  }
  .prov-block {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px 12px;
    margin-bottom: 12px;
  }
  .prov-row {
    display: flex;
    gap: 10px;
    align-items: baseline;
    padding: 2px 0;
  }
  .prov-kind {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 1px;
    font-weight: 700;
    width: 64px;
  }
  .prov-kind.defined {
    color: #6ee7a8;
  }
  .prov-kind.modified {
    color: #eab308;
  }
  .loc {
    font-family: var(--mono);
    font-size: 12px;
  }
  h3 {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 1px;
    color: var(--muted);
    margin: 16px 0 8px;
  }
  .prop {
    padding: 6px 0;
    border-bottom: 1px solid #1a1f2b;
  }
  .prop-line {
    display: flex;
    gap: 8px;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .prop-name {
    font-family: var(--mono);
    font-size: 12.5px;
    color: var(--accent);
  }
  .prop.deleted .prop-name,
  .prop.deleted .prop-value {
    text-decoration: line-through;
    color: var(--muted);
  }
  .prop-eq {
    color: var(--muted);
  }
  .prop-value {
    font-family: var(--mono);
    font-size: 12.5px;
    word-break: break-all;
    white-space: pre-wrap;
  }
  .prop-sites {
    margin-top: 4px;
  }
  .prop-sites .prov-row {
    padding: 1px 0;
  }
  .prop-sites .loc {
    font-size: 11.5px;
    color: #aeb6c4;
  }
  .badge.modified {
    background: #3a2f14;
    color: #eab308;
  }
  .badge {
    font-size: 10px;
    border-radius: 8px;
    padding: 0 6px;
    line-height: 16px;
  }
  .badge.removed {
    background: #3a1a1a;
    color: #f87171;
  }
  .hint {
    color: var(--muted);
    font-size: 12.5px;
    line-height: 1.5;
  }
  .warnings {
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.7;
    padding-left: 22px;
  }
  .empty {
    padding: 48px;
    max-width: 640px;
  }
  .empty ul {
    line-height: 2;
  }
  code {
    font-family: var(--mono);
    background: #1c2230;
    border-radius: 4px;
    padding: 1px 5px;
    font-size: 12px;
  }
</style>
