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
    close: 'Close',
    settings: 'Settings',
    steps: 'Steps',
  },
  settings: {
    title: 'Settings',
    loadFailed: 'The settings could not be read.',
    saveFailed: 'The settings could not be saved.',
    output: 'Where to save the results',
    nextToOriginals: 'In a folder next to the original photos',
    folderName: 'Folder name',
    /** Also the app's default (crates/puw-app/src/settings.rs). */
    defaultFolderName: 'Without watermark',
    alwaysFolder: 'Always save to...',
    chooseFolder: 'Choose a folder...',
    detail: 'Detail vs speed',
    best: 'Best detail',
    bestHelp: 'Best detail. Takes about three times as long as Faster.',
    balanced: 'Balanced',
    balancedHelp: 'A middle ground.',
    faster: 'Faster',
    fasterHelp: 'Quickest, with a little less detail in large photos.',
    device: 'Processor or graphics card',
    useGraphicsCard: 'Use the graphics card when it works (faster)',
    useProcessor: 'Use the processor only',
    /** Macs only, when the processor was chosen. */
    macProcessorChosen: 'The processor is used because you chose it.',
    useNeuralEngine: 'Use Neural Engine',
    savedWatermarks: 'Saved watermarks',
    about: 'About',
    advanced: 'Advanced',
    removeData: 'Remove downloaded data',
    removeDataTitle: 'Remove downloaded data?',
    removeDataIntro: 'This removes:',
    removeDataItems: [
      'The watermark remover the app downloaded (about 690 MB)',
      'Your saved watermarks',
    ],
    removeDataKeeps: 'Your settings stay. The app downloads the watermark remover again before its next use.',
    removeDataConfirm: 'Remove',
    removeDataFailed: 'The downloaded data could not be removed.',
  },
  saved: {
    title: 'Saved watermarks',
    empty: 'No saved watermarks yet. When a set is done, choose "Save this watermark for next time".',
    savedOn: (date: string) => `Saved ${date}`,
    rename: 'Rename',
    renameOne: (name: string) => `Rename ${name}`,
    newName: (name: string) => `New name for ${name}`,
    save: 'Save',
    emptyName: 'Type a name for the saved watermark.',
    delete: 'Delete',
    deleteOne: (name: string) => `Delete ${name}`,
    deleteTitle: (name: string) => `Delete "${name}"?`,
    deleteBody: 'The saved watermark cannot be brought back.',
    loadFailed: 'The saved watermarks could not be read.',
    renameFailed: 'The saved watermark could not be renamed.',
    deleteFailed: 'The saved watermark could not be deleted.',
  },
  about: {
    title: 'About PhotoUnwatermark',
    version: (version: string) => `Version ${version}`,
    responsibleUse: 'Intended for photos you own or have permission to edit.',
    credits: 'Credits and licences',
    finder: 'Watermark finder: watermark-remover by DevynLabs (christophernavas/watermark-remover), Apache License 2.0.',
    remover: 'Watermark remover: LaMa (big-lama) by Samsung Research (advimman/lama), Apache License 2.0.',
    removerWeights:
      'The LaMa weights have no licence of their own; their authors point to the Apache License 2.0. If they publish other terms for the weights, those terms apply.',
    faces: 'Face finder: YuNet from OpenCV Zoo, MIT License.',
    app: 'PhotoUnwatermark itself: MIT License.',
    newVersions: 'New versions are on the',
    releases: 'Releases page',
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
    savedWatermarks: 'Saved watermarks',
    drop: 'Drop photos or a folder here',
    dropActive: 'Let go to add them',
    choosePhotos: 'Choose photos...',
    chooseFolder: 'Choose a folder...',
    count: (n: number) => photos(n),
    remove: (name: string) => `Remove ${name}`,
    skipped: (n: number) => `${n === 1 ? '1 file was' : `${n} files were`} not added:`,
    skippedItem: (name: string, reason: string) => `${name} - ${reason}`,
    offer: (name: string) => `These look like ${name}. Use it?`,
    yes: 'Yes',
    no: 'No',
    find: 'Find the watermark →',
    removeFailed: 'The photo could not be removed from the list.',
    addFailed: 'The photos could not be added.',
  },
  finding: {
    neutral: 'Looking for the watermark...',
    reading: 'Reading your photos...',
    looking: (photo: number, total: number) => `Looking for the watermark... photo ${photo} of ${total}`,
    failed: 'The watermark could not be found.',
  },
  checkWatermark: {
    help: 'We found the watermark and painted it red. Use the Brush to paint over anything we missed and the Eraser to clean up anything that is not watermark.',
    preview: 'Preview on this photo',
    looksRight: 'Looks right →',
    /** Step 3 opens one photo in the same editor. */
    photoHelp: 'Fix the watermark area on this photo only.',
    savePhoto: 'Save this photo',
    emptyArea: 'Paint the watermark area first.',
    trying: 'Trying it on this photo...',
    backToEditing: 'Back to editing',
    strip: 'Photos from the set',
    stripPhoto: (n: number, total: number) => `Photo ${n} of ${total}`,
    photoFailed: 'The photo could not be shown.',
    areaFailed: 'The watermark area of this photo could not be read.',
    tryAgain: 'Try again',
    previewFailed: 'The preview did not work.',
    confirmFailed: 'The watermark area could not be used.',
    saveFailed: 'The photo could not be saved.',
  },
  tools: {
    brush: 'Brush',
    eraser: 'Eraser',
    rectangle: 'Rectangle',
  },
  editor: {
    /** The painting surface, for screen readers. */
    photo: 'Photo with the watermark area in red',
    toolbar: 'Painting tools',
    tools: 'Tools',
    /** A tool or button name with its keyboard shortcut, shown on hover: "Brush (B)". */
    withKey: (name: string, key: string) => `${name} (${key})`,
    /** The keys shown in those hints; the undo keys depend on the system. */
    keys: {
      brush: 'B',
      eraser: 'E',
      rectangle: 'R',
      size: '[ ]',
      undo: (mac: boolean) => (mac ? 'Cmd+Z' : 'Ctrl+Z'),
      redo: (mac: boolean) => (mac ? 'Shift+Cmd+Z' : 'Ctrl+Y'),
      zoomOut: '-',
      zoomIn: '+',
      actualSize: '1',
      fit: '0',
      magnifier: 'M',
    },
    size: 'Size',
    sizeValue: (px: number) => `${px} px`,
    undo: 'Undo',
    redo: 'Redo',
    zoomOut: 'Zoom out',
    zoomOutGlyph: '-',
    zoomIn: 'Zoom in',
    zoomInGlyph: '+',
    actualSize: '100%',
    actualSizeHint: 'Show the photo at its actual size',
    fit: 'Fit',
    fitHint: 'Show the whole photo',
    zoom: (percent: number) => `${percent}%`,
    magnifier: 'Magnifier',
    magnification: 'How much the magnifier enlarges',
    times: (n: number) => `${n}x`,
    spaceHint: 'Hold Space to hide red',
  },
  beforeAfter: {
    before: 'Before',
    after: 'After',
    /** The handle that is dragged to compare. */
    divider: 'Before and after',
  },
  checkAll: {
    /** While the watermark is looked for in every photo. */
    finding: (done: number, total: number) => `Finding the watermark in all photos... ${done} of ${total}`,
    findingFailed: 'The watermark could not be looked for in all photos.',
    loadFailed: 'The photos could not be shown.',
    tickFailed: 'The change could not be kept.',
    list: 'Photos',
    /** Shown while a ticked photo still has a "?". */
    needLookHint: 'Open or untick the photos with a ?',
    loading: 'Loading the photos...',
    empty: 'There are no photos to check.',
    refreshFailed: 'The photos were checked, but the list could not be shown.',
    /** The tile button's name: the photo and what we know about it. */
    openPhoto: (name: string, status: 'unsure' | 'notFound' | 'edited' | null) =>
      status === null
        ? `Open ${name}`
        : `Open ${name}, ${status === 'unsure' ? 'not sure the watermark was found' : status === 'notFound' ? 'watermark not found' : 'checked'}`,
    include: (name: string) => `Include ${name}`,
    noAreaTip: 'Open this photo and paint the watermark area first.',
    badgeUnsure: '?',
    badgeUnsureLabel: 'Not sure the watermark was found',
    badgeNotFound: 'Not found',
    badgeEdited: 'Checked',
    removeFrom: (n: number) => `Remove watermark from ${photos(n)}`,
    /** "Found in X of Y photos. Z need a look." The second sentence only when Z > 0. */
    summary: (found: number, total: number, needLook: number) => {
      const first = `Found in ${found} of ${photos(total)}.`;
      if (needLook === 0) return first;
      return `${first} ${needLook} ${needLook === 1 ? 'needs' : 'need'} a look.`;
    },
  },
  remove: {
    /** Before the first progress report. */
    starting: 'Getting ready...',
    photo: (n: number, total: number) => `Photo ${n} of ${total}`,
    /** From the seconds the app expects the rest to take; null until the first photo is done. */
    timeLeft: (seconds: number | null) => {
      if (seconds === null) return 'Working out the time left...';
      if (seconds < 60) return 'Less than a minute left';
      const minutes = Math.ceil(seconds / 60);
      return `About ${minutes} ${minutes === 1 ? 'minute' : 'minutes'} left`;
    },
    currentPhoto: 'The photo being worked on',
    finishedPhoto: 'The photo without the watermark',
    checking: 'Checking where to save the results...',
    /** `folders` is the list of folders, joined. */
    cannotSave: (folders: string) =>
      `The app cannot save next to these photos: ${folders}. Choose a folder for the results.`,
    chooseFolder: 'Choose a folder...',
    checkFailed: 'The app could not check where to save the results.',
    folderFailed: 'That folder could not be used.',
    failed: 'The watermark could not be removed.',
    stop: 'Stop',
    stopTitle: 'Stop after discarding the current photo?',
    keptNote: 'Finished photos are kept.',
    keepGoing: 'Keep going',
    stopping: 'Stopping...',
    /** After "Stop and quit", or a close after Stop. */
    quitting: 'Stopping... the app will close when the current photo is finished.',
    ok: 'OK',
    quitTitle: 'Removal is still running. Stop and quit?',
    stopAndQuit: 'Stop and quit',
    /** Shown for the rest of the run when the fast hardware stopped working. */
    fellBack: (mac: boolean) =>
      `The ${mac ? 'Neural Engine' : 'graphics card'} stopped working, so the app continues on the processor.`,
  },
  done: {
    openFolder: 'Open folder',
    saveWatermark: 'Save this watermark for next time',
    startNewSet: 'Start a new set',
    saved: (n: number, folder: string) => `${photos(n)} saved in '${folder}'`,
    nothingSaved: 'No photos were saved.',
    stopped: (n: number) => `Stopped. ${n === 1 ? '1 photo was' : `${n} photos were`} finished.`,
    failedTitle: (n: number) => `${photos(n)} could not be finished:`,
    failedItem: (name: string, reason: string) => `${name} - ${reason}`,
    results: 'Results',
    position: (n: number, total: number) => `Photo ${n} of ${total}`,
    previous: 'Previous photo',
    next: 'Next photo',
    previousGlyph: '←',
    nextGlyph: '→',
    name: 'Name for the saved watermark',
    savedWatermark: 'Saved',
    saveFailed: 'The watermark could not be saved.',
    openFailed: 'The folder could not be opened.',
    newSetFailed: 'A new set could not be started.',
  },
  device: {
    /** Windows, with the graphics card. */
    gpu: 'Using graphics card (fast)',
    /** Apple silicon Macs. */
    neuralEngine: 'Using Neural Engine (fast)',
    cpu: 'Using processor (slower) - why?',
    /** Settings only; the steps show nothing before the models load. */
    notStarted: 'Not started yet',
    /** The mark on a chip that opens an explanation (hidden from screen readers). */
    infoGlyph: 'i',
    whyTitle: 'Where the work runs',
    showDetails: 'Show details',
    hideDetails: 'Hide details',
    fixFailed: 'That did not work. Please try again.',
    fixes: {
      downloadHelper: (megabytes: number) => `Download GPU helper (${megabytes} MB)`,
      tryAgain: 'Try again',
      useProcessor: 'Use processor anyway',
      useGraphicsCard: 'Use graphics card',
      useNeuralEngine: 'Use Neural Engine',
    },
  },
} as const;
