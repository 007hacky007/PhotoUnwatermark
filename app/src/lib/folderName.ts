// Which names the folder next to the originals can have: the app's own rule
// (crates/puw-app/src/settings.rs, `valid_folder_name`), checked here so the person hears
// about a bad name at once.

/** Characters Windows does not allow in names, and the path separators. */
const FORBIDDEN = /[<>:"/\\|?*\u0000-\u001f\u007f]/;

/** Names Windows keeps for devices, also with an extension ("CON.txt"). */
const DEVICES = /^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$/i;

/** True when `name` can be a folder name on every system the app runs on. */
export function validFolderName(name: string): boolean {
  if (name === '' || name === '.' || name === '..') return false;
  if (name.endsWith('.') || name.endsWith(' ')) return false;
  if (FORBIDDEN.test(name)) return false;
  return !DEVICES.test((name.split('.')[0] ?? '').trimEnd());
}
