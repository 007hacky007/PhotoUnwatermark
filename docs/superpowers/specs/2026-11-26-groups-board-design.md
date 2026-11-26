# Groups first: the app sorts photos by watermark, the person fixes the groups - design

Date: 2026-11-26. Status: approved by the owner. Flow and screens agreed in conversation, with browser mockups (kept
in the gitignored .superpowers/brainstorm/ folder: groups-board.html, groups-board-v2.html, add-and-choose.html).
Replaces the flow of steps 2 and 3 in
docs/superpowers/specs/2026-11-25-watermark-groups-design.md (peel-off groups, step 2 strip, step 3 grid, single-photo
mode). Everything else in that document and in the binding spec
docs/superpowers/specs/2026-11-23-photounwatermark-design.md still holds where this document is silent.

## Why

The owner tested the peel-off groups on the Mac and found the flow backwards. In their words: "I have different
photos which may share the same watermark or similar watermark (same watermark but at different place on the photo)
or entirely different watermark. I'd expect the app to scan the photos, try to detect the watermark, group together
photos with same watermarks, so I don't have to paint over and over for these photos - the drawing will be used for
all the photos in such group. Same drawing. I'll then fix the drawing by hand and check the groups and resolve
ungrouped photos. Either by putting them manually in existing groups or by painting them individually (like new
groups)."

Problems they hit with the current build:
- It was not clear that painting in step 2 changes one drawing shared by all photos.
- Group membership could not be fixed by hand where it is decided, and step 3 was confusing: contradictory labels
  ("No group yet" on a ticked photo, no label on the one grouped photo), a misleading main button ("Find the next
  watermark in these 2" for photos with the same watermark), unexplained greyed buttons.
- On a new 3-photo set the automatic placement was wrong (Found 1, Unsure 1 misplaced, Not found 1). That engine fix
  runs separately (owner-test-fixes ledger); this design assumes matching can be wrong and gives the person the tools
  to correct it.
- The owner also asked whether more photos help. They do (for finding, not removal), and the owner may own many
  photos with the watermark but want only a few cleaned. So the app must ask for all of them and let the person pick
  the ones to clean separately.

## Goals

1. **The app groups first.** After photos are added, one "Finding watermarks" step sorts every photo into groups,
   one group per watermark, with no painting in between.
2. **One drawing per group, applied to every photo in it.** The groups screen makes this plain everywhere: the
   banner, the row label, every thumbnail shows the group's drawing.
3. **The person fixes the groups by drag and drop**: from "Not grouped" into a group, between groups, back to
   "Not grouped", or onto a "new group" row.
4. **Drawings can be reused across groups**: "Copy drawing from..." pastes another group's (or a saved watermark's)
   drawing into the current group's drawing, with move, resize and turn before it is added.
5. **Learn from all, clean only some.** Add photos asks for every photo with the watermark and shows per photo size
   whether there are enough. A separate step after the groups chooses which photos get cleaned.

## Non-goals

- Automatic search for a turned (rotated) watermark. Turning is manual (the two Move drawing tools and "Copy drawing
  from...").
- Keeping groups across an app restart (session state, as today).
- A photo in two groups. A group's drawing may hold several pieces, so a photo with two watermarks is one group
  whose drawing covers both.

## Anti-goals (failure even if it works)

- A photo marked found with the drawing in the wrong place, or on a photo without the watermark, without the person
  being shown it. Unsure must be visible and must be looked at before cleaning.
- The first guess on every photo. On a Windows PC without a usable graphics card it costs 40-55 s per photo (hours for
  200 photos). The first guess runs only on small samples (see "How the groups are made").
- Photos silently dropped: every photo ends in a group or in "Not grouped", and every count on screen adds up.
- A screen that freezes or looks done while work runs.
- The banned words (mask, mark, unmark and inflections) in the UI; "watermark", "drawing" and "group" are the nouns.

## Constraints

- All Plan 2 rules hold: copy only in app/src/lib/copy.ts, api.ts the only Tauri importer, no Unicode dashes, plain
  words, no real photos or real photo names in the repo, Core ML on Apple silicon, CPU fallback, one job at a time
  with per-call tickets, Stop on every long job. One exception to "one job at a time" (owner's choice): finding runs
  in the background while the person works on the groups screen; plain board calls (moves, painting, spots, picks)
  run alongside it, every other job waits for it.
- Speed budget (Windows VM, CPU only, 1536x1024): first guess about 40-55 s per sample photo; matching about 0.2 s per
  photo with a clean drawing. On Apple silicon the first guess is about 3 s per 24 MP photo.
- More photos help finding only for photos of the same pixel size: the set layer (median of edges across photos)
  needs at least 3 photos of one size, is fragile at 3-4, solid from 8, best from 12. Removal does not use other
  photos.

## Approaches considered

1. **First guess on every photo, then cluster.** Most thorough, but hours on a PC without a graphics card. Rejected
   for cost.
2. **Repeated sample and match, up front (chosen).** The first guess runs on a few photos, matching (cheap) finds
   that watermark on all others, the photos found form a group; the leftovers are sampled again, until nothing new
   turns up. Same result on screen as 1 at the cost of one sample per group. Risk: a watermark on only one or two
   photos may never be sampled; those photos end in "Not grouped", where the person paints them as a new group.
3. **Today's peel-off, improved.** The person starts each next group by hand. Rejected by the owner: groups are not
   visible together and the person paints group after group.

## The flow (what the person sees)

Steps: **1 Add photos, 2 Finding watermarks, 3 Check the groups, 4 Choose photos to clean, 5 Remove, 6 Done.**

### 1 Add photos

- Heading "Add every photo you have with this watermark", then: "Also the ones you do not want cleaned. The app
  learns what the watermark looks like by comparing photos: the more photos with the same watermark, the better it
  finds it. You pick which photos to clean in step 4." (wording may be polished in copy.ts, the message must stay).
- Under the drop area, one line per photo pixel size with a bar and a verdict: fewer than 3: "Too few to compare.
  Add N more."; 3-7: "Enough to start. Add N more for good results." (N = 8 - count); 8-11: "Good"; 12 or more:
  "Very good". A note says photos are compared only with photos of the same size. Upright size counts (EXIF
  rotation applied), as the set layers do.
- The saved-watermark offer ("These look like X. Use it?") is removed from this screen; saved watermarks are tried
  automatically in step 2 (below).
- Button: "Find watermarks".

### 2 Finding watermarks (until the first group exists)

- One progress bar and a live line, as below. As soon as the first group exists (a saved watermark found, or the
  first round's group), the app opens the groups screen by itself and finding goes on in the background (owner's
  choice, added after approval: "If anything happens in the background, let the user know with visible progress and
  live state update"). Back returns to Add photos and stops and drops the finding.
- How the groups are made (see the next section).

### Finding in the background (owner's choice)

- While finding goes on, the groups screen shows a progress strip above the rows: a bar, the live line ("Finding
  watermarks: found 2 so far. Group 1: 140 photos. Group 2: 52 photos. Still looking at 38 photos.") and "Stop
  looking" (photos not yet sorted go to "Not grouped"). When finding ends, the strip says so briefly ("Finding done:
  3 groups, 12 photos not grouped.") and then goes away; when the round cap stopped it, it says that instead and
  stays until closed.
- The board updates live: new groups appear as rows, photos move from the "Still looking" count into rows, counts
  change as they happen. A new group or photo that arrives gets a short highlight so the change is seen. Nothing the
  person is doing is interrupted: the current group, the photo in the editor, a drag in progress, a selection and
  the editor's view stay as they are.
- While finding runs the person can paint, erase, use both Move drawing tools, drag photos between groups, pick the
  current group and look at photos. Finding never moves a photo the person has placed or settled, and never changes
  a group's shape. A photo the person dragged while finding was about to sort it stays where the person put it.
- Actions that need the app's full attention wait while finding runs and say why next to them ("Waiting for the
  search to finish"): "Look for this watermark in the not grouped photos", "Preview on this photo", "Copy drawing
  from..." with a saved watermark, Add more photos, and "Next: choose photos to clean".
- Every background job in the app follows the same rule: visible progress and a live state update on the screen
  that shows its results, never a silent change.

### 3 Check the groups

Layout (mockup groups-board.html): the editor on top, rows below.

- **Rows.** One row per group, in creation order, then "Not grouped", then a dashed row "Drop photos here to start
  a new group". Each group row's label shows a small picture of the group's drawing, "Group N", the photo count and
  whether it has a drawing yet. Groups made from a saved watermark carry its name.
- **Thumbnails** in a group row show the group's drawing on the photo. Unsure photos carry a "?" badge. Not grouped
  photos show no drawing.
- **The editor** shows the current group (click a row to make it current) on one of its photos (click a thumbnail to
  show it). Banner: "Group N: one drawing for all K photos in this row. Painting on any of them changes it on every
  one of them." Tools: Hand (the pan tool, today called Move; key H), Brush, Eraser, Rectangle, Undo/Redo, zoom, Fit,
  Magnifier, "Move drawing on this photo", "Move drawing on all photos", "Copy drawing from...", "Preview on this
  photo". Scroll, pinch and keys as today.
- **The two Move drawing tools** (owner's request: "Two different tools for that. Because sometimes it may be useful
  to move the drawing for all photos in the group too"). Both show the drawing in a box on the photo on screen: drag
  inside to move, corners to resize (proportions kept by default, free with Shift), a round handle to turn.
  - "Move drawing on this photo" changes only this photo's spot. Its thumbnail then says "moved by you", and "Put it
    back where it was found" undoes it.
  - "Move drawing on all photos" applies the same move, resize and turn to every photo of the group, each relative to
    its own spot (a photo whose spot was moved by hand gets the same change on top).
  - Painting (Brush, Eraser, Rectangle) never moves anything: it changes the one shape on every photo, at each
    photo's spot.
- **Drag and drop.** Drag a thumbnail to another group row, to "Not grouped" or to the dashed row (which creates a new
  empty group, makes it current and shows the dropped photo). Several photos can be selected (Cmd/Ctrl-click,
  Shift-click for a range) and dragged together. Drop targets highlight. A photo moved into a group shows that
  group's drawing at once. Dropping onto a group settles any "?" on that photo. Everything is undoable as a move
  (Undo on the board, separate from drawing undo is the builder's call).
- **Copy drawing from...** lists the other groups that have a drawing and the saved watermarks, each with a small
  picture. Picking one shows that drawing over the photo in a box: drag inside to move, corners to resize (keeping
  proportions by default, free with Shift), a round handle to turn. "Add to this group's drawing" merges it
  (union) into the current group's drawing; "Cancel" drops it. It is a copy: later edits to either group do not
  affect the other.
- **Find more photos for this group.** A button on the current group ("Look for this watermark in the N not grouped
  photos") runs matching of the group's drawing over "Not grouped" and moves the photos found into the group (with
  "?" where unsure). Useful after painting a new group. Builder's call where the button sits.
- **Moving on.** "Next: choose photos to clean" is disabled while any group still holds "?" photos or has no
  drawing, with the reason next to it ("3 photos marked ? need a look: click them or drag them out"). Photos in
  "Not grouped" do not block; they cannot be cleaned (step 4 says so).
- An empty group (all photos dragged out) disappears unless it is current; its number is not reused.

### 4 Choose photos to clean

- Heading "Which photos should be cleaned?", then "Photos you do not pick were still used to find the watermark;
  they are left as they are."
- One block per group with "N of K picked", "Pick all", "Pick none", and its thumbnails (click to toggle, picked
  photos ticked, unpicked faded). All photos are picked when the step is first opened; later visits keep the
  choices.
- Not grouped photos are listed at the end as "Not grouped: N photos, not cleaned. Back to put them in a group."
- Button "Clean N photos" (disabled at 0), Back to step 3 keeps everything.

### 5 Remove, 6 Done

As today, over the picked photos only, each with its group's drawing in the group's spot. Progress, Stop, close
guard, the re-run rule. Done lists failures; photos not picked and not grouped photos are not listed as failures.
Done keeps one save row per group with a drawing (rules as today: a group from a saved watermark that was not
edited gets no row; an edited one offers "<saved name> (edited)").

## Rules the builder must keep

- **A group is one shape, with a spot per photo** (owner's choice). The shape is stored in the coordinates of the
  group's reference photo. Each photo of the group has its own spot: move, size and turn of the shape on that photo.
  Matching sets the spot (move and size; turn stays 0); the Move drawing tools change it. Painting on any photo
  changes the shape, carried back through that photo's spot, and shows on every photo at its spot.
- **Same watermark in another spot is the same group.** When matching finds a group's watermark elsewhere or at
  another size, the photo joins the group with its own spot. A set where the watermark moves on every photo is one
  group.
- **A photo of another size dropped into a group by hand** gets the reference spot scaled to its size (relative to
  width and height, shape proportions kept); the exact rule is the builder's call. A photo dropped in where
  matching did not find the watermark gets the same rule and a "?" until it is shown in the editor.
- **Pieces found apart.** When matching places the pieces of a multi-piece shape separately (they did not match
  together), the photo joins with "?" and the spot of its largest piece; the person fixes it with "Move drawing on
  this photo" or by dragging the photo out. It is never shown as found.
- A photo is in at most one group or in "Not grouped".
- Group numbers are 1, 2, 3 in creation order and never reused within a set.
- Unsure photos are never cleaned without the person having looked: a "?" is settled by showing that photo in the
  editor or by dragging it.
- **Editing never re-sorts photos.** Painting or moving changes the shape or spots only; membership, spots found by
  matching and settled "?" stay as they are. More matching runs only when the person asks ("Look for this watermark
  in the N not grouped photos"), and it only adds photos from "Not grouped" (with their own spots); it never moves a
  photo the person placed.
- The Plan 2 no-op rule applies per group: a removal result stays valid while the group's shape and that photo's spot
  are unchanged; otherwise the photo is cleaned again.
- **Adding photos later.** Add photos stays reachable from the board ("Add more photos"). New photos are matched
  against the existing groups (they join with their own spots, "?" where unsure); the rest go through the finding
  rounds of the next section. Groups and hand work are kept. Back from step 2 while finding runs stops it and
  returns to Add photos; groups found so far are kept only when finding was started from the board.

## How the groups are made (Finding watermarks)

1. **Saved watermarks first (cheap).** Each saved watermark is matched over all photos; photos found go into a group
   named after it (one group per saved watermark and spot). Unsure photos join with "?".
2. **Then repeat on what is left**, while photos remain and the last round made a group:
   - Build the set layers from the remaining photos (per size, as today) and run the first guess on a small sample
     of them (as many as today's first guess uses; the builder picks the sample to cover the remaining sizes, the
     largest size group first).
   - Take the best sample result as the round's drawing (the reference photo's guess, after the speck filter, as
     today), match it over all remaining photos, and form a group from the photos found (unsure with "?").
     The reference photo itself joins the group.
   - The other sampled photos whose own first guess (already paid for) found something and that the round's drawing
     did not find are tried next round with their own guess as the drawing before any new sample is taken.
   - A sampled photo whose guess is found on no other photo becomes a one-photo group with its guess drawn and a
     "?" (it cost a first guess; the person may drag it out).
   - A round in which no sampled photo's first guess finds anything ends the search.
3. Whatever is left goes to "Not grouped".
4. A cap on rounds (builder's call, at least 5) keeps a PC without a graphics card from running for hours; the
   progress line says when the cap stopped the search.
5. "Found 2 watermarks so far" counts groups.

**Time budget (owner's choice, Windows added after approval).** Finding for about 200 photos of 24 MP must finish
within about 5 minutes, on a mix of 2-3 watermarks, both on the owner's Mac (Apple silicon, Core ML) and on a Windows
PC without a usable graphics card (the Windows test VM is the reference). The owner finds even 5 minutes long, so the
groups screen must open early: the first group within about 40 s on the Mac and about 2 minutes on the Windows PC.
On a PC without a usable graphics card the first guess may be lighter (fewer scales and segmenters, fewer sample
photos per round) and matching may run on smaller copies, at the cost of rougher first drawings there; which
levers and how far is the builder's call, measured against the synthetic sets' recall so the loss is known and
recorded. If the budget cannot be met, the builder stops and reports instead of relaxing it. Measured today, matching costs about 4 s per 24 MP
photo per drawing tried, and a full search on a photo without the watermark 7.8 s, so this needs engine speed work
first, which is allowed to trade some first-spot precision (the person fixes spots on the board): for example
matching on smaller copies of the photos, reading and decoding each photo once for all drawings and rounds, trying
the fast same-spot test before the full search, and trying saved watermarks on a sample first. Timings go in
docs/notes as aggregates.

## What changes in today's app

- Steps 2 and 3 of the peel-off flow (CheckWatermark strip, CheckAll grid, "Find the next watermark in these N",
  "Leave them out", "Use a saved watermark...", single-photo mode, the confirm dialog about leftovers) are replaced by
  the groups screen and the choose step.
- The Saved watermarks screen keeps rename and delete (opened from Settings); its "Use for these photos" button
  goes away, because saved watermarks are tried automatically and offered in "Copy drawing from...".
- The pan tool is renamed Hand.

## What is left to the builder

- Data structures (session groups, membership, reference photo per group, board undo).
- How drag and drop is built in Svelte 5 (pointer events, keyboard alternative: select photos and a "Move to group"
  menu, needed for accessibility).
- Exact copy within the rules above; banned-word and plain-text tests as today.
- Sample size per round and the round cap; the tolerance for "same spot"; the scaling rule for other sizes.
- Test design; measured timings for finding on the synthetic sets and on the owner's real sets (aggregates only in
  docs/notes).

## Calls made on the owner's behalf (recorded for review)

1. Choosing photos to clean comes after the groups (owner unsure; reasons given: finding uses all photos anyway,
   choosing from sorted groups is easier, a helper photo in a wrong group can still spoil that group's drawing).
2. All photos start picked in step 4.
3. (Owner's choice, recorded here for completeness) one shape per group with a spot per photo, and two Move drawing
   tools: this photo, all photos.
4. Saved watermarks are tried automatically first and offered in "Copy drawing from..."; the "Use for these photos"
   button and the Add photos offer go away.
5. Unsure photos block moving to step 4 until looked at or dragged; not grouped photos do not block.
6. "Look for this watermark in the not grouped photos" exists per group.
7. Copy drawing resizes keeping proportions by default (Shift for free).
8. Editing never re-sorts photos; extra matching only on request and only from "Not grouped".
9. A sampled photo whose guess matches nothing else becomes a one-photo group with "?"; the other sampled guesses
   are tried before new samples.
10. Pieces placed apart make the photo "?" with the largest piece's spot.
11. Photos can be added from the board; they are matched against existing groups first.
12. The engine fix for wrong placements (owner-test-fixes) lands before this work; this design builds on its found /
    unsure results and the spots it reports.
