// Every sentence, label and button text the app shows. Keep to plain words: no jargon
// (see src/test/plainText.ts), hyphen-minus only; the arrow → and "..." are fine.

/** The whole model download, in megabytes (1 MB = 1,000,000 bytes). Matches the welcome text. */
export const DOWNLOAD_MEGABYTES = 690;

const photos = (n: number) => `${n} ${n === 1 ? 'photo' : 'photos'}`;

export const copy = {
  appName: 'PhotoUnwatermark',
  /** The progress line: "1 Add photos > 2 Check the watermark > ...". */
  steps: ['Add photos', 'Check the watermark', 'Check all photos', 'Remove', 'Done'],
  common: {
    back: 'Back',
    cancel: 'Cancel',
    settings: 'Settings',
    steps: 'Steps',
  },
  settings: {
    title: 'Settings',
  },
  welcome: {
    intro: 'PhotoUnwatermark removes watermarks from a whole set of photos in a few clicks.',
    needsDownload:
      'The app needs to download its watermark remover once (about 690 MB). Your photos never leave this computer.',
    download: 'Download',
    pause: 'Pause',
    resume: 'Resume',
    pausedAt: (done: number) => `Paused at ${done} MB of ${DOWNLOAD_MEGABYTES} MB.`,
    tryAgain: 'Try again',
    downloading: 'Downloading the watermark remover...',
    progress: (done: number) => `${done} MB of ${DOWNLOAD_MEGABYTES} MB`,
    failed: 'The download did not finish.',
  },
  loading: {
    finder: 'Getting the watermark finder ready...',
    remover: 'Getting the watermark remover ready...',
  },
  addPhotos: {
    offer: (name: string) => `These look like ${name}. Use it?`,
  },
  checkWatermark: {
    help: 'We found the watermark and painted it red. Use the Brush to paint over anything we missed and the Eraser to clean up anything that is not watermark.',
    preview: 'Preview on this photo',
    looksRight: 'Looks right →',
  },
  tools: {
    brush: 'Brush',
    eraser: 'Eraser',
    rectangle: 'Rectangle',
  },
  checkAll: {
    removeFrom: (n: number) => `Remove watermark from ${photos(n)}`,
    /** "Found in X of Y photos. Z need a look." The second sentence only when Z > 0. */
    summary: (found: number, total: number, needLook: number) => {
      const first = `Found in ${found} of ${photos(total)}.`;
      if (needLook === 0) return first;
      return `${first} ${needLook} ${needLook === 1 ? 'needs' : 'need'} a look.`;
    },
  },
  done: {
    openFolder: 'Open folder',
    saveWatermark: 'Save this watermark for next time',
    startNewSet: 'Start a new set',
  },
  device: {
    gpu: 'Using graphics card (fast)',
    cpu: 'Using processor (slower) - why?',
    fixes: {
      downloadHelper: (megabytes: number) => `Download GPU helper (${megabytes} MB)`,
      tryAgain: 'Try again',
      useProcessor: 'Use processor anyway',
    },
  },
} as const;
