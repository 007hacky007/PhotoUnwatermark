// One typed function per Rust command. The only module that talks to Tauri directly, so
// screen tests can replace it with `vi.mock('../lib/api')`. Failed commands reject with a
// `UserError`.

import { Channel, convertFileSrc, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl, revealItemInDir as reveal } from '@tauri-apps/plugin-opener';
import { firstTicket, tag } from './tickets';
import type {
  AddResult,
  AppStatus,
  DetectOutcome,
  DeviceFix,
  DeviceStatus,
  DownloadProgress,
  EditSize,
  GroupsState,
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
  StripPlacement,
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

// Long jobs. Each call gets its own ticket (`ticketOf(call)` in tickets.ts), and a stop names
// a ticket, so it reaches the job it was meant for and never another one. The app runs one long
// job at a time, and a job started while a stopped one is still finishing waits for it. Screens
// stop a job with `stopJob` (jobs.svelte.ts).

let tickets = firstTicket();

/** Runs `call` under a new ticket and notes the ticket on its promise. */
function job<T>(call: (ticket: number) => Promise<T>): Promise<T> {
  const ticket = ++tickets;
  return tag(call(ticket), ticket);
}

/**
 * Asks the job with ticket `job` (or the download with it) to stop; that job's own call then
 * rejects. Resolves once the app has taken the stop.
 */
export const cancelJob = (job: number | undefined) => invoke<void>('cancel_job', { job });

// Start and download

export const appStatus = () => invoke<AppStatus>('app_status');

export const downloadModels = (onEvent: (p: DownloadProgress) => void) =>
  job((t) => invoke<void>('download_models', { onEvent: channel(onEvent), job: t }));

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

export const addPaths = (paths: string[]) => job((t) => invoke<AddResult>('add_paths', { paths, job: t }));

export const removePhoto = (id: PhotoId) => job((t) => invoke<void>('remove_photo', { id, job: t }));

export const clearPhotos = () => invoke<void>('clear_photos');

export const listPhotos = () => invoke<PhotoInfo[]>('list_photos');

/** Looks for a saved watermark on the photos. */
export const suggestSaved = () => job((t) => invoke<Suggestion | null>('suggest_saved', { job: t }));

export const detect = (onEvent: (e: ProgressEvent) => void) =>
  job((t) => invoke<DetectOutcome>('detect', { onEvent: channel(onEvent), job: t }));

/** Starts the next group from saved watermark `id`, reading no photo; resolves with its number. */
export const useSaved = (id: string) => job((t) => invoke<number>('use_saved', { id, job: t }));

// Step 2: the watermark area

export const editSize = (id: PhotoId) => invoke<EditSize>('edit_size', { id });

/** The photo's area at edit size: one byte per pixel (0 or 255), row by row. */
export async function getArea(id: PhotoId): Promise<Uint8Array> {
  return new Uint8Array(await invoke<ArrayBuffer>('get_area', { id }));
}

/** The area step 2 starts from, at edit size like `getArea`: the step-2 guess for a photo in no group. */
export async function getStartArea(id: PhotoId): Promise<Uint8Array> {
  return new Uint8Array(await invoke<ArrayBuffer>('start_area', { id }));
}

/** Where the shared drawing on photo `reference` lands on photo `id`. */
export const placeOn = (reference: PhotoId, id: PhotoId) =>
  job((t) => invoke<StripPlacement>('place_on', { reference, id, job: t }));

/** The photos step 2 shows for the current group, and the one it opens first. */
export const groupSample = () => invoke<DetectOutcome>('group_sample');

/** Hands the area the editor shows to the app (edit size; non-zero bytes are the area). */
export const putDraft = (id: PhotoId, width: number, height: number, area: Uint8Array) =>
  invoke<void>('put_draft', area, {
    headers: { 'x-photo-id': String(id), 'x-width': String(width), 'x-height': String(height) },
  });

/** Forgets the area sent for a photo whose editor was left without saving. */
export const dropDraft = (id: PhotoId) => invoke<void>('drop_draft', { id });

export const confirmSetArea = (id: PhotoId) => job((t) => invoke<void>('confirm_set_area', { id, job: t }));

/** Makes the area of photo `id` the next group's watermark area; that group becomes the current one. */
export const confirmNewGroup = (id: PhotoId) => job((t) => invoke<void>('confirm_new_group', { id, job: t }));

export const preview = (id: PhotoId, onEvent: (e: ProgressEvent) => void) =>
  job((t) => invoke<void>('preview', { id, onEvent: channel(onEvent), job: t }));

// Step 3: all photos

export const matchAll = (onEvent: (e: ProgressEvent) => void) =>
  job((t) => invoke<MatchSummary>('match_all', { onEvent: channel(onEvent), job: t }));

export const photoStates = () => invoke<PhotoState[]>('photo_states');

/** Every group, and where the photos in none stand. */
export const groupsState = () => invoke<GroupsState>('groups');

/** Removes every group and every photo's result. */
export const clearGroups = () => job((t) => invoke<void>('clear_groups', { job: t }));

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
  job((t) => invoke<RemoveReport>('remove', { onEvent: channel(onEvent), job: t }));

/** Shows the file in the system's file manager (Finder, Explorer), in its folder. */
export const revealItemInDir = (path: string) => reveal(path);

// The window

/**
 * Calls `hold` when the person closes the window: the window stays open only when it returns
 * true (and closes when it throws). Resolves with the function that stops it.
 */
export async function onCloseRequested(hold: () => boolean): Promise<() => void> {
  return getCurrentWindow().onCloseRequested((event) => {
    let held = false;
    try {
      held = hold();
    } catch {
      // The window must stay closable.
    }
    if (held) event.preventDefault();
  });
}

/**
 * Calls `onQuit` when the app was asked to quit (Cmd+Q) during a removal; the app has stayed
 * open and quits with `closeWindow`. Resolves with the function that stops it.
 */
export async function onQuitRequested(onQuit: () => void): Promise<() => void> {
  return listen('quit-requested', () => onQuit());
}

/** Closes the window (and so the app) without asking. */
export const closeWindow = () => getCurrentWindow().destroy();

// Saved watermarks

/** Saves group `group`'s watermark under `name` and returns its id. */
export const saveWatermark = (group: number, name: string) => invoke<string>('save_watermark', { group, name });

export const defaultWatermarkName = () => invoke<string>('default_watermark_name');

export const listSaved = () => invoke<SavedInfo[]>('list_saved');

export const renameSaved = (id: string, name: string) => invoke<void>('rename_saved', { id, name });

export const deleteSaved = (id: string) => invoke<void>('delete_saved', { id });

// Settings

export const getSettings = () => invoke<Settings>('get_settings');

export const setSettings = (settings: Settings) => invoke<void>('set_settings', { settings });

// Processing device

export const deviceStatus = () => invoke<DeviceStatus>('device_status');

/** Applies a fix from the "why?" dialog; the models load again when next needed. */
export const deviceFix = (fix: DeviceFix) => invoke<void>('device_fix', { fix });

// Links

/** Where new versions are published (the opener only allows pages of the app's repository). */
export const RELEASES_URL = 'https://github.com/007hacky007/PhotoUnwatermark/releases';

/** Opens the releases page in the browser. */
export const openReleasesPage = () => openUrl(RELEASES_URL);
