// The data the app's Rust side sends and takes, mirroring its types field by field.

export type PhotoId = number;

/** What a failed command rejects with: a sentence to show, and the raw text for "Show details". */
export interface UserError {
  message: string;
  details: string;
  /** Files the app downloaded are missing: offer the download again. */
  missingData?: boolean;
}

export interface AppStatus {
  modelsReady: boolean;
  /** Bytes still to download (files already complete are not counted). */
  missingBytes: number;
  platform: 'macos' | 'windows' | 'linux' | string;
  version: string;
}

export interface DownloadProgress {
  doneBytes: number;
  totalBytes: number;
  file: string;
}

export type LoadPurpose = 'finder' | 'remover';

/** What a long job is doing. Fractions run from 0 to 1. */
export type ProgressEvent =
  | { phase: 'loadingModels'; fraction: number; which: LoadPurpose }
  | { phase: 'reading'; fraction: number }
  | { phase: 'looking'; done: number; total: number }
  | { phase: 'matching'; done: number; total: number }
  | {
      phase: 'removing';
      done: number;
      total: number;
      photo: PhotoId;
      photoFraction: number;
      secondsLeft: number | null;
    }
  | { phase: 'preview'; fraction: number }
  | { phase: 'fellBackToProcessor'; reason: string };

/** Why a photo cannot be used: it is no longer there, or it cannot be opened. */
export type PhotoProblem = 'gone' | 'damaged';

/** Width and height are the upright size. */
export interface PhotoInfo {
  id: PhotoId;
  name: string;
  folder: string;
  width: number;
  height: number;
  /** Set once reading the photo failed. */
  problem?: PhotoProblem | null;
}

export interface Skipped {
  name: string;
  reason: string;
}

export interface AddResult {
  added: PhotoInfo[];
  skipped: Skipped[];
}

export interface Suggestion {
  id: string;
  name: string;
}

export interface DetectOutcome {
  sample: PhotoId[];
  representative: PhotoId;
}

/** Size of the image a photo's area is drawn on. */
export interface EditSize {
  width: number;
  height: number;
}

export interface MatchSummary {
  found: number;
  unsure: number;
  notFound: number;
  total: number;
}

export type AreaStatus = 'found' | 'unsure' | 'notFound' | 'edited' | 'pending' | 'unreadable';

export interface PhotoState {
  id: PhotoId;
  status: AreaStatus;
  included: boolean;
  reviewed: boolean;
  hasArea: boolean;
  /** The group its result or own area came from. */
  group: number | null;
  /** In group `group`: ticked, readable, with a non-empty area, and not an unsure photo nobody opened. */
  member: boolean;
}

/** How sure a strip placement is. */
export type Placed = 'found' | 'unsure' | 'notFound';

/**
 * Where the shared drawing lands on a strip photo: the reference photo's edit-image pixel
 * (x, y) goes to (x * scaleX + dx, y * scaleY + dy) on this photo's edit image. All 0 for
 * 'notFound'.
 */
export interface StripPlacement {
  status: Placed;
  scaleX: number;
  scaleY: number;
  dx: number;
  dy: number;
}

/** A watermark group as step 3 and Done show it. */
export interface GroupInfo {
  number: number;
  /** Its members. */
  photos: number;
  /** The saved watermark it was started from. */
  savedName: string | null;
  /** That saved watermark's area was changed in step 2. */
  edited: boolean;
  /** It can be saved as a watermark for next time. */
  canSave: boolean;
}

/** Every group, and where the photos in none stand. */
export interface GroupsState {
  groups: GroupInfo[];
  /** The current group (the last one made). */
  current: number | null;
  /** The current group's matching has finished. */
  matched: boolean;
  /** Readable photos in no group. */
  leftovers: number;
  /** Photos that cannot be read. */
  unreadable: number;
}

export interface OutputCheck {
  ok: boolean;
  /** Folders that cannot be written to. */
  unwritable: string[];
}

export interface Done {
  id: PhotoId;
  /** Where the result was saved. */
  output: string;
}

export interface Failed {
  id: PhotoId;
  name: string;
  reason: string;
}

/** A photo in no group, which a removal leaves alone. */
export interface LeftOut {
  id: PhotoId;
  name: string;
}

export interface RemoveReport {
  done: Done[];
  failed: Failed[];
  /** The photos with no group when the run started, in list order. */
  leftOut: LeftOut[];
  cancelled: boolean;
}

export interface SavedInfo {
  id: string;
  name: string;
  /** When it was saved (RFC 3339, UTC). */
  created: string;
}

export type OutputRule = { nextToOriginals: { folderName: string } } | { folder: { path: string } };

export type Detail = 'best' | 'balanced' | 'faster';

export interface Settings {
  output: OutputRule;
  detail: Detail;
  preferGpu: boolean;
  /** Measured seconds to load each model; the app keeps its own values when saving. */
  loadSeconds: Record<string, number>;
}

/** The pictures the `puw` protocol serves. */
export type ImageRoute = 'thumb' | 'grid' | 'edit' | 'preview' | 'original' | 'result' | 'saved';

/** What the device chip shows: graphics card, processor, or nothing loaded yet. */
export type DeviceState = 'gpu' | 'cpu' | 'notStarted';

/** A button the "why?" dialog offers. */
export type DeviceFix = 'tryAgain' | 'useProcessor' | 'useGraphicsCard' | { downloadHelper: { bytes: number } };

export interface DeviceStatus {
  state: DeviceState;
  /** The sentence behind "why?", when there is one. */
  reason: string | null;
  /** The raw text for "Show details". */
  details: string | null;
  fixes: DeviceFix[];
}
