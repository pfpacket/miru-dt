import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { LoadResult } from './types';

export async function pickDtsFile(): Promise<string | null> {
  const picked = await open({
    multiple: false,
    filters: [{ name: 'Device tree source', extensions: ['dts', 'dtsi', 'dtso'] }],
  });
  return typeof picked === 'string' ? picked : null;
}

export async function pickDtbFile(): Promise<string | null> {
  const picked = await open({
    multiple: false,
    filters: [{ name: 'Device tree blob', extensions: ['dtb', 'dtbo'] }],
  });
  return typeof picked === 'string' ? picked : null;
}

export function loadDts(path: string, includeDirs: string[]): Promise<LoadResult> {
  return invoke<LoadResult>('load_dts', { path, includeDirs });
}

export function loadDtb(path: string): Promise<LoadResult> {
  return invoke<LoadResult>('load_dtb', { path });
}

export function loadLive(): Promise<LoadResult> {
  return invoke<LoadResult>('load_live', {});
}
