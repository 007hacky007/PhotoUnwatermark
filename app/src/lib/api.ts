// One typed function per Rust command. The only module that talks to Tauri directly, so
// screen tests can replace it with `vi.mock('../lib/api')`. Failed commands reject with a
// `UserError`.

import { Channel, convertFileSrc, invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import type {
  AddResult,
  AppStatus,
  DetectOutcome,
  DownloadProgress,
  EditSize,
  ImageRoute,
  MatchSummary,
  OutputCheck,
  PhotoId,
  PhotoInfo,
  PhotoState,
  ProgressEvent,
  RemoveReport,
  SavedInfo,
  Settings,
  Suggestion,
} from './types';

function channel<T>(onEvent: (event: T) => void): Channel<T> {
  const ch = new Channel<T>();
  ch.onmessage = onEvent;
  return ch;
}

/** URL of a picture from the `puw` protocol. Pass `version` to load a changed picture again. */
export function imageUrl(route: ImageRoute, id: PhotoId | string, version?: number): string {
  const url = convertFileSrc(`${route}/${id}`, 'puw');
  return version === undefined ? url : `${url}?v=${version}`;
}

// Start and download

export const appStatus = () => invoke<AppStatus>('app_status');

export const downloadModels = (onEvent: (p: DownloadProgress) => void) =>
  invoke<void>('download_models', { onEvent: channel(onEvent) });

/** Asks the running job (or the download) to stop; the job's own call then rejects. */
export const cancelJob = () => invoke<void>('cancel_job');

export const removeDownloadedData = () => invoke<void>('remove_downloaded_data');

// Step 1: photos

/** What the window reports while files are dragged over it. `paths` is empty except on a drop. */
export interface DragDrop {
  kind: 'hover' | 'leave' | 'drop';
  paths: string[];
}

/** Calls `onEvent` for files dragged over the window. Resolves with the function that stops it. */
export async function onDragDrop(onEvent: (e: DragDrop) => void): Promise<() => void> {
  return getCurrentWebview().onDragDropEvent(({ payload }) => {
    if (payload.type === 'enter' || payload.type === 'over') onEvent({ kind: 'hover', paths: [] });
    else if (payload.type === 'leave') onEvent({ kind: 'leave', paths: [] });
    else onEvent({ kind: 'drop', paths: payload.paths });
  });
}

/** The kinds of photo the app reads (HEIC dropped directly is reported as skipped). */
const PHOTO_EXTENSIONS = ['jpg', 'jpeg', 'png', 'tif', 'tiff', 'webp'];

/** Asks for photos; an empty list when the person cancels. */
export async function choosePhotos(): Promise<string[]> {
  const picked = await open({
    multiple: true,
    directory: false,
    filters: [{ name: 'Photos', extensions: PHOTO_EXTENSIONS }],
  });
  return picked ?? [];
}

/** Asks for a folder; `null` when the person cancels. */
export async function chooseFolder(): Promise<string | null> {
  return open({ multiple: false, directory: true });
}

export const addPaths = (paths: string[]) => invoke<AddResult>('add_paths', { paths });

export const removePhoto = (id: PhotoId) => invoke<void>('remove_photo', { id });

export const clearPhotos = () => invoke<void>('clear_photos');

export const listPhotos = () => invoke<PhotoInfo[]>('list_photos');

export const suggestSaved = () => invoke<Suggestion | null>('suggest_saved');

export const detect = (onEvent: (e: ProgressEvent) => void) =>
  invoke<DetectOutcome>('detect', { onEvent: channel(onEvent) });

export const useSaved = (id: string, onEvent: (e: ProgressEvent) => void) =>
  invoke<DetectOutcome>('use_saved', { id, onEvent: channel(onEvent) });

// Step 2: the watermark area

export const editSize = (id: PhotoId) => invoke<EditSize>('edit_size', { id });

/** The photo's area at edit size: one byte per pixel (0 or 255), row by row. */
export async function getArea(id: PhotoId): Promise<Uint8Array> {
  return new Uint8Array(await invoke<ArrayBuffer>('get_area', { id }));
}

/** Hands the area the editor shows to the app (edit size; non-zero bytes are the area). */
export const putDraft = (id: PhotoId, width: number, height: number, area: Uint8Array) =>
  invoke<void>('put_draft', area, {
    headers: { 'x-photo-id': String(id), 'x-width': String(width), 'x-height': String(height) },
  });

export const confirmSetArea = (id: PhotoId) => invoke<void>('confirm_set_area', { id });

export const preview = (id: PhotoId, onEvent: (e: ProgressEvent) => void) =>
  invoke<void>('preview', { id, onEvent: channel(onEvent) });

// Step 3: all photos

export const matchAll = (onEvent: (e: ProgressEvent) => void) =>
  invoke<MatchSummary>('match_all', { onEvent: channel(onEvent) });

export const photoStates = () => invoke<PhotoState[]>('photo_states');

export const openPhoto = (id: PhotoId) => invoke<void>('open_photo', { id });

export const savePhotoArea = (id: PhotoId) => invoke<void>('save_photo_area', { id });

export const setIncluded = (id: PhotoId, included: boolean) =>
  invoke<void>('set_included', { id, included });

export const removalBlockers = () => invoke<PhotoId[]>('removal_blockers');

// Steps 4 and 5: removal and the results

export const checkOutput = () => invoke<OutputCheck>('check_output');

/** A folder for this set's results instead of the usual one; `null` goes back to the usual one. */
export const setOutputOverride = (path: string | null) =>
  invoke<void>('set_output_override', { path });

export const remove = (onEvent: (e: ProgressEvent) => void) =>
  invoke<RemoveReport>('remove', { onEvent: channel(onEvent) });

// Saved watermarks

/** Saves the set's watermark under `name` and returns its id. */
export const saveWatermark = (name: string) => invoke<string>('save_watermark', { name });

export const defaultWatermarkName = () => invoke<string>('default_watermark_name');

export const listSaved = () => invoke<SavedInfo[]>('list_saved');

export const renameSaved = (id: string, name: string) => invoke<void>('rename_saved', { id, name });

export const deleteSaved = (id: string) => invoke<void>('delete_saved', { id });

// Settings

export const getSettings = () => invoke<Settings>('get_settings');

export const setSettings = (settings: Settings) => invoke<void>('set_settings', { settings });
