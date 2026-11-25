# Watermark groups and one shared drawing - design

Date: 2026-11-25. Status: approved by the owner in conversation (flow and approach); the owner then handed
remaining decisions to the builder ("make reasonable calls and record them"). Extends the binding spec
docs/superpowers/specs/2026-11-23-photounwatermark-design.md; where this document is silent, that spec holds.

## Why

Testing the Plan 2 app on a real set, the owner selected three photos with the same watermark and expected the app
to treat them as one watermark with one drawing for the whole batch. Instead, step 2 showed each sample photo with
its own red area, and only the photo on screen at "Looks right" counted. The owner also wants to handle batches
that mix several different watermarks, and to pick a saved watermark directly instead of waiting for the automatic
offer.

## Goals

1. **One shared drawing per watermark.** In step 2 the person draws the watermark once. The strip shows that same
   drawing placed on each sample photo; painting on any sample changes the one shared drawing.
2. **Watermark groups by peeling off (owner's choice).** The app handles one watermark at a time: find it, check it,
   find it in all photos; the photos where it was found form a group. Photos left over can start the next group
   ("Find the next watermark in these N photos"), until none are left or the person leaves them out. One removal run
   at the end handles every group, each photo with its group's drawing.
3. **"Use for these photos" on saved watermarks (owner's choice).** A saved watermark can be chosen directly; it
   starts a group without the first guess. The automatic offer ("These look like X. Use it?") stays and does the
   same.

## Non-goals

- Sorting all photos into groups up front (runs the slow first guess on every photo; rejected by the owner in
  favour of peeling off).
- A photo with two different watermarks at once. One photo belongs to at most one group. (Workaround: run the
  results through the app again.)
- Keeping groups across an app restart (session state, as today).
- Moving a photo directly from one group to another; unticking returns it to "no group yet" (below), which covers it.

## Anti-goals (failure even if it works)

- Any extra pass of the first guess over all photos. The first guess runs only on a small sample: of all photos for
  group 1, of the leftovers for later groups.
- Photos silently dropped: every photo ends in a group, in "Leave them out", or as "could not be opened", and the
  counts on screen say so.
- A screen that freezes or a step that looks done while work runs (Plan 2 rules hold).
- Jargon: the banned words (mask, mark, unmark and inflections) stay banned; "watermark area" and "group" are the
  nouns. "Group" is plain English and allowed.

## Constraints

- All Plan 2 rules hold: copy only in app/src/lib/copy.ts, api.ts the only Tauri importer, no Unicode dashes, plain
  words, no real photos or real photo names in the repo, Core ML on Apple silicon, CPU fallback, one job at a time
  with per-call tickets.
- Speed budget (measured on the Windows VM, processor only, 1536x1024): first guess about 40-55 s per sample photo;
  matching about 0.2 s per photo with a clean drawing; removal 12 s (Faster) to 40 s (Best) per photo. On Apple
  silicon all are several times faster. A group must cost one sample guess (or none with a saved watermark) plus one
  matching pass over the ungrouped photos.

## Approaches considered

1. **Group first:** run the first guess on every photo, cluster by watermark, then check each cluster. Shows all
   groups at once, but costs a first guess per photo (hours for 200 photos on a PC without a usable graphics card).
   Rejected for that cost.
2. **Peel off groups (chosen by the owner):** one watermark at a time; matching (cheap) decides membership; the first
   guess runs only on a sample of what is left. Groups appear one after another.
3. **Next watermark after removal:** the leftovers go back to step 2 after a removal run. Cheapest, but removal
   happens several times and the person must wait between groups. The owner chose 2 instead.

## The design (what the person sees)

Steps keep their numbers and names (1 Add photos, 2 Check the watermark, 3 Check all photos, 4 Remove, 5 Done).
While more than one group exists, steps 2 and 3 show the current group in their heading line, for example
"Group 2: check the watermark".

### Step 1 - Add photos

As today. The automatic offer ("These look like X. Use it?") remains. New: the "Saved watermarks" screen, opened
from Add photos while photos are added, shows "Use for these photos" on each saved watermark. Choosing one (or "Yes"
on the offer) starts group 1 from that watermark: the app matches it right away and goes to step 3 for that group
(the spec's rule that the person "still lands in step 2 to confirm" is replaced: step 3's grid is the confirmation,
and step 2 is one Back away). Builder's call: skipping step 2 here saves a click per group and step 3 shows where the
saved watermark landed on every photo.

### Step 2 - Check the watermark (one shared drawing)

- The editor shows the group's reference photo (the most confident sample, as today) and the strip of sample photos.
- There is exactly one drawing for the group. It is kept in the reference photo's coordinates. On another strip
  photo it is shown where the app finds it on that photo (same placement as step 3 matching uses), and edits made on
  that photo are carried back to the shared drawing through the same placement.
- If the drawing cannot be placed on a strip photo (not found there), that strip photo shows a plain note ("The
  watermark was not found on this photo.") and painting there is off; the person can still look at it.
- The first guess for the shared drawing is the reference photo's guess (after the Plan 2 speck filter). Builder's
  call: other samples' guesses are not merged in, because a wrong merge adds false areas and the strip now shows the
  drawing on every sample for checking anyway.
- "Preview on this photo" works on any strip photo, using the shared drawing at its placement there.
- "Looks right" confirms the shared drawing for the group. Undo history is per group and covers edits made on any
  strip photo.

### Step 3 - Check all photos (per group)

- Matching runs the current group's drawing over every photo that has no group yet (never over photos already in an
  earlier group, never over unreadable photos).
- The grid shows all photos. Tiles of earlier groups show a small group label ("Group 1") and keep their state;
  tiles of the current group show found / unsure / not found as today; photos with no group yet show "No group yet".
- Current group membership: found photos and opened-and-saved ("Checked") photos. Unsure photos block moving on
  (to the next group or to Remove) until opened or unticked, as today. Unticking a photo of the current group returns
  it to "no group yet".
- Below the grid, when photos with no group are left:
  - summary line, for example "Group 1: 140 photos. Group 2: 52 photos. 38 photos have no group yet.";
  - "Find the next watermark in these 38" (primary when leftovers exist): runs the first guess on a sample of the
    leftovers only (batch layers from the leftovers only), then step 2 for the new group. If the first guess finds
    nothing, the person lands in step 2 with an empty drawing and the existing empty-area hint.
  - "Leave them out": the leftovers are not processed; they are listed on Done as "left out" (not as failures).
  - Before running the first guess on the leftovers, the automatic saved-watermark offer is tried on them (cheap);
    "Yes" starts the next group from it as in step 1.
- "Remove watermark from N photos" is available whenever no unsure photo blocks it; N counts every grouped, ticked,
  readable photo. Pressing it with leftovers present asks once: "38 photos have no group yet. Remove from the 192
  photos in groups and leave the 38 out?" with "Remove" and "Back".
- Opening a tile of an earlier group opens it in single-photo mode for that photo (its own area, as today).
- Back from step 3 goes to step 2 of the current group with its drawing kept. Going back further than that is not
  offered; earlier groups are edited through their tiles (single-photo mode) only. Builder's call: re-editing a whole
  earlier group's drawing would re-assign photos across groups and is not worth the complexity now.

### Step 4 - Remove

One run over all grouped, ticked photos; each photo uses its own area (from its group's drawing at its matched
placement, or its own saved edit). Progress, Stop, close guard and the re-run rule (skip unchanged finished photos)
as today. Group order does not matter for the run; photos run in list order.

### Step 5 - Done

- Heading and failure list as today; left-out photos get their own short list ("Left out: 38 photos").
- One save row per group: name field (default "Watermark from <date>", with " (group N)" appended when there are
  several groups) and "Save this watermark for next time". "Saved" shows per group.
- "Start a new set" clears all groups.

### Saved watermarks screen

- Opened from Add photos with photos present: each row gets "Use for these photos" (starts the next group from it;
  with no group yet that is group 1). Opened from Settings: no such button (rename and delete as today).
- After "Use", matching runs and step 3 shows the group.

## Behaviour details the builder must keep

- A photo belongs to at most one group. Assigning happens only when a group's matching or a person's save puts it
  there; leaving it (unticking, or matching for a later group skipping it) is explicit.
- Group numbering is 1, 2, 3 in creation order and never reused within a set.
- The confirmed-area no-op rule (Plan 2 fix A2) applies per group.
- Cancel/Back during the leftovers' first guess returns to step 3 with groups unchanged.
- Removing a photo from the list on Add photos (only possible before step 2) is unchanged; adding photos after groups
  exist is not possible without "Start a new set" (Add photos is behind step 2). Builder's call: keeps group
  membership consistent.

## What is left to the builder

- Data structures for groups in the app session (puw-app) and how the UI holds the current group.
- How a strip photo's placement is computed and cached (reuse step-3 matching).
- Exact copy wording within the rules above (plain words, through copy.ts, banned-word tests).
- Test design; measured timings for one extra group on the synthetic sets (record aggregates in docs/notes).

## Calls made by the builder on the owner's behalf (recorded for review)

1. Saved watermark "Use"/"Yes" goes straight to step 3 for that group (not step 2).
2. The shared first drawing is the reference photo's guess only; other samples' guesses are not merged.
3. Earlier groups cannot be re-edited as a whole; only per photo.
4. Unticking a current-group photo returns it to "no group yet".
5. One removal run for all groups at the end.
6. Photos cannot be added once groups exist (start a new set instead).
7. Leftover photos can be left out explicitly; they are listed as "left out" on Done, not as failures.

## Answers to the builder check (builder's calls, recorded for review)

8. **Saved watermark for group 2 and later.** Step 3 gets "Use a saved watermark..." next to "Find the next
   watermark in these N" whenever photos with no group are left. It opens the Saved watermarks screen in "choose"
   mode for the leftovers; "Use for these photos" matches that watermark over the leftovers only and returns to step
   3 with the new group. So the Saved watermarks screen has three entries: from Settings (manage only), from Add
   photos (starts group 1), from step 3 (starts the next group).
9. **When a group exists, and Back from step 2.** A group is created (and gets its number) at "Looks right" in step
   2, or at "Use" of a saved watermark. Until then it is a draft. Back from step 2:
   - draft of group 2 or later: returns to step 3, the draft is thrown away, leftovers unchanged, its number is not
     used up;
   - group 1 (draft or confirmed): returns to Add photos as today; photos can be added or removed there; if any group
     was confirmed, all groups are cleared (Plan 2 rule: an unchanged photo list keeps the step-2 drawing, a changed
     one detects again). The "Photos cannot be added once groups exist" rule above means: not without going back to
     Add photos, which clears groups;
   - a confirmed group k > 1 reopened from step 3 (Back from step 3): Back returns to step 3 with the group kept as
     it was.
10. **Membership, exactly.** A photo is in a group when it is ticked, readable and has an area from that group:
    found by matching, an unsure photo that was opened (opening settles it, as today), or saved in single-photo mode
    ("Checked"). Not found, unticked and unreadable photos are in no group.
11. **Unticking earlier-group photos.** Allowed on any grouped tile. The photo leaves its group at once (its area is
    dropped) and becomes "no group yet"; the next group's matching includes it. Its checkbox then behaves like a
    "no group yet" tile (disabled with the reason "Not in a group yet") until a later group finds it.
12. **Unsure placement on a strip photo.** Three strip states: found (drawing shown, painting on), unsure (drawing
    shown, painting off, note "Not sure the watermark is here on this photo."), not found (no drawing, painting off,
    note as above). Only found placements carry edits back to the shared drawing.
13. **"Leave them out"** is an action: it goes straight to Remove (step 4) without the confirm dialog, the leftovers
    marked as left out. The confirm dialog only appears when "Remove watermark from N photos" is pressed while
    leftovers exist.
14. **Done and saved-watermark groups.** A group that came from a saved watermark and was not edited gets no save row
    (it is already saved). If its drawing was edited in step 2, its row offers to save it as a new watermark with the
    default name "<saved name> (edited)"; the existing saved watermark is never overwritten.
