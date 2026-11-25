// The plain-language rule for everything the app shows: no jargon, no Unicode dashes.

export const BANNED = {
  /** Never shown as whole words, in any case ("watermark", "watermarks", "watermarked" are fine). */
  words: [
    'mask',
    'masks',
    'masked',
    'masking',
    'mark',
    'marks',
    'marked',
    'marking',
    'unmark',
    'unmarked',
  ],
  /** Never shown anywhere inside a word, in any case. */
  parts: ['inpaint', 'segment', 'threshold', 'dilat', 'onnx', 'tensor'],
  /**
   * Hyphen, non-breaking hyphen, figure dash, en and em dash, bar, minus sign, and the small
   * em dash, small and full-width hyphen-minus.
   */
  dashes: /[\u2010-\u2015\u2212\ufe58\ufe63\uff0d]/u,
} as const;

/** Attributes whose text people see or hear. */
const SHOWN_ATTRIBUTES = ['title', 'aria-label', 'placeholder', 'alt'];

/** What breaks the rule in `text`, one entry per problem; empty when the text is fine. */
export function plainTextProblems(text: string): string[] {
  const problems: string[] = [];
  const lower = text.toLowerCase();
  for (const word of lower.split(/[^\p{L}\p{N}]+/u)) {
    if ((BANNED.words as readonly string[]).includes(word)) problems.push(`word "${word}"`);
  }
  for (const part of BANNED.parts) {
    if (lower.includes(part)) problems.push(`"${part}"`);
  }
  const dash = text.match(BANNED.dashes);
  if (dash) problems.push(`dash U+${dash[0].codePointAt(0)!.toString(16).toUpperCase()}`);
  return problems;
}

/** Throws when the text of `el` or a shown attribute of it or any element inside breaks the rule. */
export function assertPlainText(el: Element): void {
  const found: string[] = [];
  const check = (where: string, text: string) => {
    for (const p of plainTextProblems(text)) found.push(`${where}: ${p} in "${text}"`);
  };
  check('text', el.textContent ?? '');
  for (const node of [el, ...el.querySelectorAll('*')]) {
    for (const name of SHOWN_ATTRIBUTES) {
      const value = node.getAttribute(name);
      if (value !== null) check(`${node.tagName.toLowerCase()}[${name}]`, value);
    }
  }
  if (found.length > 0) throw new Error(`Not plain text:\n${found.join('\n')}`);
}
