import { describe, expect, it } from 'vitest';
import { copy } from '../lib/copy';
import { assertPlainText, plainTextProblems } from './plainText';

/** Sample arguments for the functions that do not take numbers or names. */
const SAMPLES: Record<string, unknown[][]> = {
  'copy.groups.line': [
    [[], 1],
    [[{ number: 1, photos: 1 }], 0],
    [[{ number: 1, photos: 140 }, { number: 2, photos: 52 }], 38],
    [[{ number: 1, photos: 5 }], 0, 1],
    [[{ number: 1, photos: 140 }, { number: 2, photos: 52 }], 38, 2],
  ],
};

/** Every string in `copy`, with each function called for a few sample values. */
function allTexts(value: unknown, path = 'copy'): [string, string][] {
  if (typeof value === 'string') return [[path, value]];
  if (typeof value === 'function' && SAMPLES[path]) {
    return SAMPLES[path].map((args) => [`${path}(${JSON.stringify(args)})`, String(value(...args))] as [string, string]);
  }
  if (typeof value === 'function') {
    const samples = [0, 1, 2, 37, 118, 1024];
    return samples.map((n) => {
      const args = Array.from({ length: value.length }, (_, i) => n + i);
      return [`${path}(${args.join(', ')})`, String(value(...args))] as [string, string];
    });
  }
  if (Array.isArray(value)) return value.flatMap((v, i) => allTexts(v, `${path}[${i}]`));
  if (value !== null && typeof value === 'object') {
    return Object.entries(value).flatMap(([k, v]) => allTexts(v, `${path}.${k}`));
  }
  throw new Error(`${path} is neither text nor a function`);
}

describe('copy', () => {
  it('noBannedWords', () => {
    const texts = allTexts(copy);
    expect(texts.length).toBeGreaterThan(20);
    for (const [path, text] of texts) {
      expect(text.trim(), path).not.toBe('');
      expect(plainTextProblems(text), `${path}: ${text}`).toEqual([]);
    }
  });

  it('exactSpecCopy', () => {
    const line = copy.steps.map((s, i) => `${i + 1} ${s}`).join(' > ');
    expect(line).toBe('1 Add photos > 2 Check the watermark > 3 Check all photos > 4 Remove > 5 Done');
    expect(copy.checkWatermark.help).toBe(
      'We found the watermark and painted it red. Use the Brush to paint over anything we missed and the Eraser to clean up anything that is not watermark.',
    );
    expect(copy.tools).toEqual({ move: 'Move', brush: 'Brush', eraser: 'Eraser', rectangle: 'Rectangle' });
    expect(copy.checkWatermark.preview).toBe('Preview on this photo');
    expect(copy.checkWatermark.looksRight).toBe('Looks right →');
    expect(copy.checkAll.removeFrom(118)).toBe('Remove watermark from 118 photos');
    expect(copy.done.openFolder).toBe('Open folder');
    expect(copy.done.saveWatermark).toBe('Save this watermark for next time');
    expect(copy.done.startNewSet).toBe('Start a new set');
    expect(copy.addPhotos.offer('Studio')).toBe('These look like Studio. Use it?');
    expect(copy.checkAll.summary(118, 120, 2)).toBe('Found in 118 of 120 photos. 2 need a look.');
    expect(copy.loading.remover).toBe('Getting the watermark remover ready...');
    expect(copy.loading.finder).toBe('Getting the watermark finder ready...');
    expect(copy.device.gpu).toBe('Using graphics card (fast)');
    expect(copy.device.cpu).toBe('Using processor (slower) - why?');
    expect(copy.device.fixes.downloadHelper(120)).toBe('Download GPU helper (120 MB)');
    expect(copy.device.fixes.tryAgain).toBe('Try again');
    expect(copy.device.fixes.useProcessor).toBe('Use processor anyway');

    const g = copy.groups;
    expect(g.checkHeading(2)).toBe('Group 2: check the watermark');
    expect(g.checkAllHeading(2)).toBe('Group 2: check all photos');
    expect(g.label(3)).toBe('Group 3');
    expect(g.noGroup).toBe('No group yet');
    expect(g.notInGroup).toBe('Not in a group yet');
    expect(
      g.line(
        [
          { number: 1, photos: 140 },
          { number: 2, photos: 52 },
        ],
        38,
      ),
    ).toBe('Group 1: 140 photos. Group 2: 52 photos. 38 photos have no group yet.');
    expect(g.line([{ number: 1, photos: 1 }], 1)).toBe('Group 1: 1 photo. 1 photo has no group yet.');
    expect(g.line([{ number: 1, photos: 140 }], 0)).toBe('Group 1: 140 photos.');
    expect(g.line([{ number: 1, photos: 5 }], 0, 1)).toBe('Group 1: 5 photos. 1 photo could not be opened.');
    expect(g.line([{ number: 1, photos: 5 }], 3, 2)).toBe(
      'Group 1: 5 photos. 3 photos have no group yet. 2 photos could not be opened.',
    );
    expect(g.findNext(38)).toBe('Find the next watermark in these 38');
    expect(g.findNext(1)).toBe('Find the next watermark in this photo');
    expect(g.useSaved).toBe('Use a saved watermark...');
    expect(g.leaveOut).toBe('Leave them out');
    expect(g.confirmLeftovers(38, 192)).toBe(
      '38 photos have no group yet. Remove from the 192 photos in groups and leave the 38 out?',
    );
    expect(g.confirmLeftovers(1, 192)).toBe(
      '1 photo has no group yet. Remove from the 192 photos in groups and leave it out?',
    );
    expect(g.confirmLeftovers(2, 1)).toBe(
      '2 photos have no group yet. Remove from the 1 photo in groups and leave the 2 out?',
    );
    expect(g.confirmRemove).toBe('Remove');
    expect(g.nextFailed).toBe('The next watermark could not be looked for.');

    const w = copy.checkWatermark;
    expect(w.notFoundHere).toBe('The watermark was not found on this photo.');
    expect(w.unsureHere).toBe('Not sure the watermark is here on this photo.');
    expect(w.placing).toBe('Finding the watermark on this photo...');
    expect(w.placeFailed).toBe('The watermark area could not be shown on this photo.');

    expect(copy.saved.useForThese).toBe('Use for these photos');
    expect(copy.saved.useOne('Studio')).toBe('Use Studio for these photos');
    expect(copy.saved.useFailed).toBe('The saved watermark could not be used.');

    expect(copy.done.leftOut(38)).toBe('Left out: 38 photos');
    expect(copy.done.leftOut(1)).toBe('Left out: 1 photo');
    expect(copy.done.groupName(2)).toBe('Name for the saved watermark of group 2');
    expect(copy.done.groupDefault('Watermark from 2026-11-26', 2)).toBe('Watermark from 2026-11-26 (group 2)');
    expect(copy.done.editedName('Studio')).toBe('Studio (edited)');
    expect(copy.done.savedGroup(2)).toBe('Group 2: saved');
  });

  it('countsReadNaturally', () => {
    expect(copy.checkAll.removeFrom(1)).toBe('Remove watermark from 1 photo');
    expect(copy.checkAll.summary(120, 120, 0)).toBe('Found in 120 of 120 photos.');
    expect(copy.checkAll.summary(0, 1, 1)).toBe('Found in 0 of 1 photo. 1 needs a look.');
  });
});

describe('plainText', () => {
  it('watermarkIsFineButJargonIsNot', () => {
    expect(plainTextProblems('Remove the watermark. Watermarks too.')).toEqual([]);
    expect(plainTextProblems('Edit the Mask')).not.toEqual([]);
    expect(plainTextProblems("mark it")).not.toEqual([]);
    expect(plainTextProblems('UNMARK')).not.toEqual([]);
    expect(plainTextProblems('Inpainting...')).not.toEqual([]);
    expect(plainTextProblems('ONNX model')).not.toEqual([]);
    expect(plainTextProblems('Dilated area')).not.toEqual([]);
    expect(plainTextProblems('slow \u2013 why')).not.toEqual([]);
    expect(plainTextProblems('\u22121')).not.toEqual([]);
    expect(plainTextProblems('slow - why? → ...')).toEqual([]);
    for (const word of ['masks', 'Masked', 'MASKING', 'marks', 'Marked', 'marking', 'Unmarked']) {
      expect(plainTextProblems(`Photo ${word} here`), word).not.toEqual([]);
    }
    expect(plainTextProblems('Watermarks, watermarked and WATERMARK photos')).toEqual([]);
    for (const dash of ['\ufe58', '\ufe63', '\uff0d']) {
      expect(plainTextProblems(`a ${dash} b`), dash).not.toEqual([]);
    }
  });

  it('checksShownAttributes', () => {
    const ok = document.createElement('div');
    ok.innerHTML = '<button title="Brush" aria-label="Brush">B</button><img alt="Your photo">';
    expect(() => assertPlainText(ok)).not.toThrow();
    for (const html of [
      '<button title="Mask">B</button>',
      '<button aria-label="Threshold">B</button>',
      '<input placeholder="Name \u2014 here">',
      '<img alt="tensor">',
      '<p>Segmenting...</p>',
    ]) {
      const el = document.createElement('div');
      el.innerHTML = html;
      expect(() => assertPlainText(el), html).toThrow(/Not plain text/);
    }
  });
});
