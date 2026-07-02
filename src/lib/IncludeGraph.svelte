<script lang="ts">
  import type { IncludeGraph } from './types';

  interface Props {
    graph: IncludeGraph;
    shorten: (file: string) => string;
    onopen: (file: string) => void;
  }

  let { graph, shorten, onopen }: Props = $props();

  interface Row {
    file: string;
    depth: number;
    line: number | null;
    directive: string | null;
    repeat: boolean;
  }

  function flatten(g: IncludeGraph): Row[] {
    const byFrom = new Map<string, typeof g.edges>();
    for (const e of g.edges) {
      const list = byFrom.get(e.from) ?? [];
      list.push(e);
      byFrom.set(e.from, list);
    }
    const rows: Row[] = [{ file: g.root, depth: 0, line: null, directive: null, repeat: false }];
    const expanded = new Set<string>([g.root]);
    const walk = (file: string, depth: number) => {
      for (const e of byFrom.get(file) ?? []) {
        const repeat = expanded.has(e.to);
        rows.push({ file: e.to, depth, line: e.line, directive: e.directive, repeat });
        if (!repeat) {
          expanded.add(e.to);
          walk(e.to, depth + 1);
        }
      }
    };
    walk(g.root, 1);
    return rows;
  }

  const rows = $derived(flatten(graph));
</script>

<div class="graph">
  <p class="summary">
    {graph.files.length} file{graph.files.length === 1 ? '' : 's'}, {graph.edges.length}
    include{graph.edges.length === 1 ? '' : 's'}
  </p>
  {#each rows as row, i (i)}
    <div class="file-row" style:padding-left="{row.depth * 18 + 4}px">
      {#if row.depth > 0}<span class="elbow">└─</span>{/if}
      <button
        class="file"
        class:root={row.depth === 0}
        class:repeat={row.repeat}
        title="open {row.file} in editor"
        onclick={() => onopen(row.file)}
      >
        {shorten(row.file)}
      </button>
      {#if row.directive !== null}
        <span class="edge-info">{row.directive} at line {row.line}</span>
      {/if}
      {#if row.repeat}
        <span class="repeat-mark" title="already expanded above">↺</span>
      {/if}
    </div>
  {/each}
</div>

<style>
  .summary {
    color: var(--muted, #7a8394);
    font-size: 12px;
    margin: 0 0 8px;
  }
  .file-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding-top: 2px;
    padding-bottom: 2px;
  }
  .elbow {
    color: var(--border, #2a3242);
    font-family: var(--mono, monospace);
    font-size: 12px;
  }
  .file {
    font-family: var(--mono, monospace);
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    background: none;
    border: none;
    color: inherit;
    padding: 0;
    cursor: pointer;
    text-align: left;
  }
  .file:hover {
    color: var(--accent, #7aa2ff);
    text-decoration: underline;
  }
  .file.root {
    font-weight: 600;
    color: var(--accent, #7aa2ff);
  }
  .file.repeat {
    color: var(--muted, #7a8394);
  }
  .edge-info {
    color: var(--muted, #7a8394);
    font-size: 11px;
    white-space: nowrap;
  }
  .repeat-mark {
    color: var(--muted, #7a8394);
    font-size: 11px;
  }
</style>
