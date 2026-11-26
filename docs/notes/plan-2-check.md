# Plan 2 check (manual, by the owner)

Fill in each `[ ]` with `[x]` (works) or `[!]` (problem) and a few words. Aggregates only: no
photo names, no photo descriptions. Wait times go in the line, for example "(2 s)".

Start the release binary (built with `pnpm -C app tauri build --no-bundle`):
`target/release/photounwatermark` from a terminal in the
repo root, so its messages stay visible. Nothing is installed.

Test photos:
- Synthetic sets: `target/synth/fixed`, `target/synth/random`, `target/synth/styles` (each has
  `marked/`, `clean/`, `mask/`; drop the `marked/` folder). Regenerate with
  `uv run --project tools python tools/synth/make_sets.py --out target/synth` if missing.
- Real sets (never committed): `local/real/set-a` (8 photos, same mark) and `local/real/set-b`
  (1 photo, other size).
- Fresh data folder: move `~/Library/Application Support/PhotoUnwatermark` away first and move
  it back at the end. Windows: `%LOCALAPPDATA%\PhotoUnwatermark`.

Rule for every screen: the window never freezes for more than about 200 ms (devtools
performance panel, or just watch the spinner and the cursor), and no sentence reads as
jargon (note the exact text if one does).

## 1. Welcome / first launch (fresh data folder)

- [ ] First start shows the Welcome screen with the download offer; nothing downloads by itself.
- [ ] "Download" starts the real download; progress moves, size and time left look sane.
- [ ] Interrupt the download (quit the app or disconnect the network), start again: it resumes (does not start from 0).
- [ ] Download finishes: "Add photos" opens. Total wait: ____ s.
- [ ] Quit and start again: no download offer, straight to the next screen.
- [ ] The device chip shows the real device (Mac: "Using Neural Engine (fast)" and the chip reads as clickable; click it: the explanation opens).
- [ ] Settings: Remove downloaded data asks first and then returns to Welcome (do this last, then re-download or restore the folder).

## 2. Step 1: Add photos

- [ ] Drop the folder `target/synth/fixed/marked`: thumbnails appear within ~2 s; the count is right.
- [ ] Hover with a dragged folder over the window: the drop target highlights; leaving removes it.
- [ ] Drop a folder and single photos together; add the same photo twice: duplicates are skipped with a short note.
- [ ] Drop a text file or a damaged image: it is skipped and the reason is readable.
- [ ] "x" on a photo removes it; removing the last photo clears the "saved watermark" offer.
- [ ] Portrait photos (phone-style, rotated by EXIF) show upright in the grid.
- [ ] 24 MP photos (real set-a): thumbnails appear without a frozen window.
- [ ] A photo deleted from disk after adding: the next step shows it as a problem, not a crash.
- [ ] With a saved watermark from an earlier set: the offer to use it appears and works (skips finding).
- [ ] Back and Next buttons match the step numbers shown at the top.

## 3. Step 1 to 2: Finding the watermark

- [ ] First use: the "getting ready" sentence for the finder shows; cold load ____ s, warm ____ s.
- [ ] Sentences change while it works; Cancel returns to Add photos with the photos kept.
- [ ] Detection on `fixed` finishes: ____ s. On real set-a: ____ s.
- [ ] If the device falls back to the processor, the notice appears once and the chip says why.

## 4. Step 2: Check the watermark (editor)

- [ ] The red layer sits on the watermark on `fixed` (sample photos in the strip at the bottom).
- [ ] Brush, eraser and rectangle (drag in all four directions) paint and erase where expected.
- [ ] Undo and redo by button and by Cmd/Ctrl+Z, Shift+Cmd+Z and Ctrl+Y.
- [ ] Wheel zoom and Mac trackpad pinch around the cursor; - / 100% / + / Fit; keys 0, 1, + and -. Middle-drag and Space-drag pan. The photo refits on window resize until zoomed.
- [ ] Hold Space (no drag) hides the red layer, release shows it. Space does not scroll the page or press buttons (also right after clicking a toolbar button).
- [ ] Magnifier at 2x, 4x, 8x near all four edges and corners: stays inside the box, empty outside the photo, hides off the photo and while panning, shows the brush circle or rectangle outline and the red layer.
- [ ] The brush outline follows the cursor and scales with the zoom. Painting fast on a 24 MP photo: no stutter or frozen frames.
- [ ] Strip switching keeps each photo's edits; Back to Add photos and forward again keeps them; a new detection starts fresh.
- [ ] "Try it on this photo": remover-loading sentence on first use, then "Trying it on this photo...". Preview time (24 MP real photo): ____ s (spec target about 15 s). Cancel works.
- [ ] Before/after handle works by drag and by arrow keys; the device chip shows; Back to editing keeps the undo history.
- [ ] Preview shows the new result every time (not the old picture) after editing the area again.
- [ ] "Looks right" with an empty area shows the message; with an area it goes to step 3.
- [ ] A photo whose area fails to load shows the error with "Try again".

## 5. Step 3: Check all photos

- [ ] `random` set: matching runs, a progress line moves; time ____ s. Real set-a: ____ s.
- [ ] The grid shows found / unsure / not found badges that read clearly; "Found in X" count is right.
- [ ] Scroll a set of 200+ photos (copy a set several times): smooth, images load lazily.
- [ ] A tile opens the editor; saving updates the tile image and badge; scroll position is kept; Back keeps the tile.
- [ ] Unsure photos block the Remove button until opened or unticked; the hint says what to do (wording: "Open or untick the photos with a ?").
- [ ] Not found photos show a disabled checkbox with a tooltip that explains why.
- [ ] Back during matching cancels and returns to step 2 with the area kept; reopening step 3 only matches the photos not matched yet.
- [ ] Portrait photos: the area lands on the watermark, not rotated.
- [ ] Real set-b (other size): matched or marked not found without an error.
- [ ] `styles` set: each style found or marked unsure/not found, no crash.
- [ ] Photo mode (open one photo from the grid): help line, "Save this photo", Back discards.

## 6. Step 4: Remove

- [ ] `fixed` set, all 8 photos: bar, "Photo X of Y" and time left move ("Working out the time left..." until the first photo ends); the thumbnail turns into the result.
- [ ] The `Without watermark` folder next to the photos has the results; originals are unchanged.
- [ ] Second run on the same folder: it writes again without errors and without leftover partial files.
- [ ] Per-photo time on a real 24 MP photo: Best ____ s, Faster ____ s (Settings, Detail).
- [ ] Stop midway: the dialog appears, "Stopping..." shows, Done says "Stopped. N photos were finished.", finished files are on disk, no partial files.
- [ ] Read-only output folder (for example `chmod a-w` on the photos folder): the question appears, "Choose a folder..." works, results land there, Done names that folder.
- [ ] A photo deleted after adding: it is listed as failed with a readable reason; the others finish.
- [ ] Close the window during a run: the in-app question appears; "Keep going" continues; "Stop and quit" stops, then the app closes.
- [ ] macOS: Cmd+Q during a run shows the same question; with no run it quits at once; the app menu still reads "Quit PhotoUnwatermark" with Cmd+Q.
- [ ] Click Stop, then close the window: "Stopping... the app will close in a moment." with only OK; a second close during the wait closes at once.
- [ ] Close on Add photos or Done closes at once, with no question.

## 7. Step 5: Done

- [ ] The before/after slider works by drag, buttons and Left/Right keys.
- [ ] "Open folder" reveals the first result in Finder or Explorer.
- [ ] Save the watermark under the default name: "Saved" shows; it appears in Saved watermarks; Settings then Back keeps "Saved" and the same photo.
- [ ] "Start a new set" opens an empty Add photos; the next set does not reload the models (no loading sentence).

## 8. Settings and About

- [ ] Detail (Best / Balanced / Faster) changes are kept after a restart.
- [ ] Output rule: another folder works; "next to originals" works.
- [ ] Saved watermarks: rename, delete and thumbnail work.
- [ ] Credits and licences lists the models and the LaMa wording.
- [ ] Windows only: a forced GPU failure shows the fallback notice and the chip "Using processor"; the "why" dialog fixes it.

## 9. Whole app

- [ ] No frozen window over about 200 ms at any point above (devtools performance panel).
- [ ] No jargon: list any text that reads technical (exact wording): ______________
- [ ] The release build has no console errors on any screen (CSP is enforced only in the built app).

## 10. Windows smoke (needs the CI artifact)

Prerequisite (needs a push the owner approves): run the CI workflow with "Run workflow"
(`workflow_dispatch`) and download the artifact `photounwatermark-windows-exe`. Copy the exe
to the winbench VM (upload to `C:\bench`). To skip the download, copy the three model files
into `%LOCALAPPDATA%\PhotoUnwatermark\models`; or download once from the Welcome screen.

- [ ] The exe starts and the window opens (no missing DLL, no error box).
- [ ] The synthetic `fixed` set goes through all 5 steps; result files appear in `Without watermark`.
- [ ] The device chip shows the real reason when the GPU is not used (read it, note the text).
- [ ] Toolbar buttons do not take the focus on click; the AltGr layouts' [ and ] keys change the brush size.
- [ ] Drag and drop of a folder works; Explorer opens with "Open folder".

## 11. Watermark groups

Set up: the mixed set is `target/synth/fixed/marked` plus the logo photos the timings run writes
under `local/timing/mixed/marked` (and `target/synth/negative/marked` 04 and 05 for photos with no watermark).

- [ ] Mixed set: group 1 from the `fixed` photos, then "Find the next watermark" for the logo photos.
- [ ] Step 2 of group 2: one drawing is shown on every photo in the strip, and painting on one photo changes all of them.
- [ ] Strip photos: an unsure or not-found note shows on the photo, with painting off there.
- [ ] Undo works across strip photos (it undoes the last stroke wherever it was painted).
- [ ] The group heading in step 2 and step 3 reads "group 2" for the second group.
- [ ] Earlier-group labels show on step-3 tiles, "No group yet" tiles show for the rest, and unticking a photo of an earlier group works.
- [ ] The summary line counts each photo once (in a group, no group yet, or unreadable) and "Found in X of Y" leaves unreadable photos out of Y.
- [ ] "Find the next watermark": the saved-watermark check comes first, then the wait; Back keeps everything.
- [ ] "Use a saved watermark..." and "Use for these photos" work from Add photos and from step 3; step 3 shows the result with no wait screen.
- [ ] "Leave them out" and the Remove dialog agree on which photos are left out.
- [ ] Done: the list of left-out photos shows, and there is one "Saved" row per group.
- [ ] Back from step 2 of group 1 to Add photos clears the groups and keeps the drawing.
- [ ] Label placement on small step-3 tiles: the group label does not cover the badge or the checkbox.
- [ ] Step-3 footer with four buttons: it wraps cleanly at narrow window widths.
- [ ] The saved-watermark offer in step 3: layout and wording are clear.
- [ ] Painting speed on a 24 MP photo in step 2 with the shared drawing (the whole red layer is redrawn per stroke): no lag you can feel.
- [ ] "Looks right" pressed while a strip photo is still being placed: nothing breaks and the drawing used is the one shown.
- [ ] With several groups, one "Saved" line per group on Done is easy to understand.
- [ ] Timings for the second group: ____ s (first guess plus matching).
- [ ] Real set 1 + 2 as one set, group 1 from the first guess unedited: step 2's strip shows the drawing in place on every strip photo of the mark's size (measured: 2 of 2, up to 1.2 s each on 24 MP), and step 3 groups the 8 photos of that size (measured: 6 found, 2 unsure, all in place) and not the other photo.
- [ ] Step 2 on 24 MP photos: after a stroke, switching to another strip photo shows it within about a second (measured: 0.7 s for the first photo after a change, 0.01 s for the rest).
- [ ] Stop while a strip photo is being placed (switch photos during "Finding the watermark on this photo..."): the next photo starts at once (measured: a stopped 7.8 s search ends 0.04 s after the stop).
- [ ] "Looks right" with the drawing unchanged on a 24 MP photo of a group: step 3 keeps every result and tick, and a saved watermark stays unedited.

Result summary (fill in last): macOS ____ passed / ____ problems; Windows ____ / ____.
