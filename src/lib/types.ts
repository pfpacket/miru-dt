// Mirrors the Rust model in src-tauri/src/model.rs.

export interface SourceLoc {
  file: string;
  line: number;
}

export interface Provenance {
  defined: SourceLoc;
  modified: SourceLoc[];
}

export interface DtProperty {
  name: string;
  value: string;
  deleted: boolean;
  provenance: Provenance | null;
}

export interface DtNode {
  name: string;
  labels: string[];
  properties: DtProperty[];
  children: DtNode[];
  deleted: boolean;
  provenance: Provenance | null;
}

export interface IncludeEdge {
  from: string;
  to: string;
  line: number;
  directive: string;
}

export interface IncludeGraph {
  root: string;
  files: string[];
  edges: IncludeEdge[];
}

export type SourceKind = 'dts' | 'dtb' | 'live';

export interface LoadResult {
  source: string;
  kind: SourceKind;
  tree: DtNode;
  includeGraph: IncludeGraph | null;
  warnings: string[];
}
