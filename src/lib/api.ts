import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { LoadResult } from './types';

export async function pickDeviceTreeFile(): Promise<string | null> {
  const picked = await open({
    multiple: false,
    filters: [
      { name: 'Device tree files', extensions: ['dts', 'dtsi', 'dtso', 'dtb', 'dtbo'] },
      { name: 'All files', extensions: ['*'] },
    ],
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

export function openSource(file: string, line?: number): Promise<void> {
  return invoke<void>('open_source', { file, line });
}
