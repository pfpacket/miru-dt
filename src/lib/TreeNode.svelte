<script lang="ts">
  import type { DtNode } from './types';
  import TreeNode from './TreeNode.svelte';

  interface Props {
    node: DtNode;
    path: string;
    depth: number;
    selectedPath: string | null;
    filter: string;
    onselect: (path: string, node: DtNode) => void;
  }

  let { node, path, depth, selectedPath, filter, onselect }: Props = $props();

  // Nodes start expanded down to depth 2; only the initial depth matters.
  // svelte-ignore state_referenced_locally
  let expanded = $state(depth < 2);

  function nodeMatches(n: DtNode, q: string): boolean {
    return (
      n.name.toLowerCase().includes(q) ||
      n.labels.some((l) => l.toLowerCase().includes(q)) ||
      n.properties.some(
        (p) => p.name.toLowerCase().includes(q) || p.value.toLowerCase().includes(q)
      )
    );
  }

  function subtreeMatches(n: DtNode, q: string): boolean {
    return nodeMatches(n, q) || n.children.some((c) => subtreeMatches(c, q));
  }

  const q = $derived(filter.trim().toLowerCase());
  const visible = $derived(q === '' || subtreeMatches(node, q));
  const forceOpen = $derived(q !== '' && node.children.some((c) => subtreeMatches(c, q)));
  const isOpen = $derived(forceOpen || expanded);
  const modCount = $derived(node.provenance?.modified.length ?? 0);
</script>

{#if visible}
  <div class="tree-item">
    <div class="row" class:selected={selectedPath === path} style:padding-left="{depth * 14 + 4}px">
      <button
        class="disclosure"
        disabled={node.children.length === 0}
        aria-label={isOpen ? 'collapse' : 'expand'}
        onclick={() => (expanded = !isOpen)}
      >
        {node.children.length === 0 ? '·' : isOpen ? '▾' : '▸'}
      </button>
      <button class="node-btn" class:deleted={node.deleted} onclick={() => onselect(path, node)}>
        {#each node.labels as label (label)}
          <span class="label">{label}:</span>
        {/each}
        <span class="name">{node.name}</span>
        {#if modCount > 0}
          <span class="badge modified" title="modified {modCount} time{modCount > 1 ? 's' : ''}">
            ✎{modCount}
          </span>
        {/if}
        {#if node.deleted}
          <span class="badge removed">deleted</span>
        {/if}
      </button>
    </div>
    {#if isOpen}
      {#each node.children as child, i (i)}
        <TreeNode
          node={child}
          path={path === '/' ? `/${child.name}` : `${path}/${child.name}`}
          depth={depth + 1}
          {selectedPath}
          {filter}
          {onselect}
        />
      {/each}
    {/if}
  </div>
{/if}

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: 4px;
  }
  .row:hover {
    background: var(--hover, #1d2330);
  }
  .row.selected {
    background: var(--selected, #24304a);
  }
  .disclosure {
    background: none;
    border: none;
    color: var(--muted, #7a8394);
    width: 18px;
    padding: 0;
    cursor: pointer;
    font-size: 11px;
  }
  .disclosure:disabled {
    cursor: default;
  }
  .node-btn {
    background: none;
    border: none;
    color: inherit;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 6px 2px 2px;
    cursor: pointer;
    font: inherit;
    min-width: 0;
    flex: 1;
    text-align: left;
  }
  .name {
    font-family: var(--mono, monospace);
    font-size: 12.5px;
    white-space: nowrap;
  }
  .label {
    font-family: var(--mono, monospace);
    font-size: 11.5px;
    color: var(--accent2, #4fd1c5);
    white-space: nowrap;
  }
  .badge {
    font-size: 10px;
    border-radius: 8px;
    padding: 0 6px;
    line-height: 16px;
    white-space: nowrap;
  }
  .badge.modified {
    background: #3a2f14;
    color: #eab308;
  }
  .badge.removed {
    background: #3a1a1a;
    color: #f87171;
  }
  .deleted .name {
    text-decoration: line-through;
    color: var(--muted, #7a8394);
  }
</style>
