<script lang="ts">
  import { onMount } from 'svelte';
  import { appStatus, chooseFolder, getSettings, removeDownloadedData, setSettings } from '../lib/api';
  import DeviceChip from '../lib/components/DeviceChip.svelte';
  import Dialog from '../lib/components/Dialog.svelte';
  import { copy } from '../lib/copy';
  import { validFolderName } from '../lib/folderName';
  import type { Detail, Settings, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  /** The settings as last saved. */
  let settings: Settings | null = $state(null);
  let platform = $state('');
  let error: string | null = $state(null);
  // What the controls show. Each changes as soon as it is clicked and goes back to the saved
  // value when a save fails, so a failed choice never stays on screen.
  let outputKind: 'next' | 'folder' = $state('next');
  let folderName = $state('');
  let detail: Detail = $state('best');
  let preferGpu = $state(true);
  /** Bumped when the device choice changes, so the chip asks again. */
  let chipRefresh = $state(0);
  let confirming = $state(false);
  let removing = $state(false);
  let removeError: string | null = $state(null);

  const id = $props.id();
  const details: { value: Detail; label: string; help: string }[] = [
    { value: 'best', label: copy.settings.best, help: copy.settings.bestHelp },
    { value: 'balanced', label: copy.settings.balanced, help: copy.settings.balancedHelp },
    { value: 'faster', label: copy.settings.faster, help: copy.settings.fasterHelp },
  ];

  const messageOf = (e: unknown, fallback: string) => (e as Partial<UserError> | null)?.message || fallback;
  const kindOf = (s: Settings) => ('nextToOriginals' in s.output ? 'next' : 'folder');

  function show(s: Settings) {
    settings = s;
    outputKind = kindOf(s);
    if ('nextToOriginals' in s.output) folderName = s.output.nextToOriginals.folderName;
    detail = s.detail;
    preferGpu = s.preferGpu;
  }

  // The platform comes first, so the device part never shows the wrong wording.
  onMount(async () => {
    const [system, result] = await Promise.allSettled([appStatus(), getSettings()]);
    platform = system.status === 'fulfilled' ? system.value.platform : '';
    if (result.status === 'fulfilled') show(result.value);
    else error = messageOf(result.reason, copy.settings.loadFailed);
  });

  // Saves run one after another. Once none is waiting, the controls show what was saved.
  let queue: Promise<void> = Promise.resolve();
  let pending = 0;

  function enqueue(task: () => Promise<void>): Promise<void> {
    pending++;
    const run = queue.then(task).finally(() => {
      pending--;
      if (pending === 0 && settings !== null) show(settings);
    });
    queue = run.catch(() => {});
    return run;
  }

  /** Saves `change` applied to the latest saved settings, so quick changes never undo each other. */
  function save(change: (s: Settings) => Settings): Promise<void> {
    error = null;
    return enqueue(async () => {
      if (settings === null) return;
      const next = change(settings);
      try {
        await setSettings(next);
      } catch (e) {
        error = messageOf(e, copy.settings.saveFailed);
        return;
      }
      if (next.preferGpu !== settings.preferGpu) chipRefresh++;
      settings = next;
    });
  }

  /** Reads the settings again (a fix in the "why?" dialog may have changed them). */
  function reload(): Promise<void> {
    return enqueue(async () => {
      try {
        settings = await getSettings();
      } catch (e) {
        error = messageOf(e, copy.settings.loadFailed);
      }
    });
  }

  function chooseDetail(value: Detail) {
    detail = value;
    void save((s) => ({ ...s, detail: value }));
  }

  function choosePreferGpu(value: boolean) {
    preferGpu = value;
    void save((s) => ({ ...s, preferGpu: value }));
  }

  /** The folder name saved last, or the usual one. */
  function savedFolderName(): string {
    return settings !== null && 'nextToOriginals' in settings.output
      ? settings.output.nextToOriginals.folderName
      : copy.settings.defaultFolderName;
  }

  function useNextToOriginals() {
    outputKind = 'next';
    const name = folderName.trim() || copy.settings.defaultFolderName;
    void save((s) => ({ ...s, output: { nextToOriginals: { folderName: name } } }));
  }

  function renameFolder() {
    const name = folderName.trim();
    if (!validFolderName(name)) {
      // The saved name stays, and the person hears why.
      error = copy.settings.badFolderName;
      folderName = savedFolderName();
      return;
    }
    error = null;
    folderName = name;
    if (name === savedFolderName() && settings !== null && kindOf(settings) === 'next') return;
    void save((s) => ({ ...s, output: { nextToOriginals: { folderName: name } } }));
  }

  async function pickFolder() {
    outputKind = 'folder';
    const path = await chooseFolder().catch(() => null);
    if (path === null) {
      // Cancelled: show the saved choice again.
      if (settings !== null) outputKind = kindOf(settings);
      return;
    }
    await save((s) => ({ ...s, output: { folder: { path } } }));
  }

  async function removeData() {
    removing = true;
    removeError = null;
    try {
      await removeDownloadedData();
    } catch (e) {
      removeError = messageOf(e, copy.settings.removeDataFailed);
      return;
    } finally {
      removing = false;
    }
    confirming = false;
    wizard.go('welcome');
  }

  function closeConfirm() {
    if (removing) return;
    confirming = false;
    removeError = null;
  }
</script>

<section class="settings">
  <div class="top">
    <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
    <h1>{copy.settings.title}</h1>
  </div>

  {#if error !== null}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if settings !== null}
    <fieldset>
      <legend>{copy.settings.output}</legend>
      <label class="choice">
        <input type="radio" name="{id}-output" checked={outputKind === 'next'} onchange={useNextToOriginals} />
        {copy.settings.nextToOriginals}
      </label>
      <label class="sub">
        {copy.settings.folderName}
        <input type="text" bind:value={folderName} onchange={renameFolder} disabled={outputKind !== 'next'} />
      </label>
      <label class="choice">
        <input type="radio" name="{id}-output" checked={outputKind === 'folder'} onchange={pickFolder} />
        {copy.settings.alwaysFolder}
      </label>
      {#if 'folder' in settings.output}
        <div class="sub folder">
          <span class="path">{settings.output.folder.path}</span>
          <button type="button" onclick={pickFolder}>{copy.settings.chooseFolder}</button>
        </div>
      {/if}
    </fieldset>

    <fieldset>
      <legend>{copy.settings.detail}</legend>
      {#each details as d (d.value)}
        <label class="choice">
          <input
            type="radio"
            name="{id}-detail"
            checked={detail === d.value}
            aria-describedby="{id}-{d.value}"
            onchange={() => chooseDetail(d.value)}
          />
          {d.label}
        </label>
        <p class="sub help" id="{id}-{d.value}">{d.help}</p>
      {/each}
    </fieldset>

    <fieldset>
      <legend>{copy.settings.device}</legend>
      {#if platform === 'windows'}
        <label class="choice">
          <input
            type="radio"
            name="{id}-device"
            checked={preferGpu}
            onchange={() => choosePreferGpu(true)}
          />
          {copy.settings.useGraphicsCard}
        </label>
        <label class="choice">
          <input
            type="radio"
            name="{id}-device"
            checked={!preferGpu}
            onchange={() => choosePreferGpu(false)}
          />
          {copy.settings.useProcessor}
        </label>
      {:else if platform === 'macos' && !settings.preferGpu}
        <div class="mac-choice">
          <p>{copy.settings.macProcessorChosen}</p>
          <button type="button" onclick={() => choosePreferGpu(true)}>{copy.settings.useNeuralEngine}</button>
        </div>
      {/if}
      <div class="chip">
        <DeviceChip inSettings {platform} refresh={chipRefresh} onChange={reload} />
      </div>
    </fieldset>

    <div class="links">
      <button type="button" onclick={() => wizard.go('saved')}>{copy.settings.savedWatermarks}</button>
      <button type="button" onclick={() => wizard.go('about')}>{copy.settings.about}</button>
    </div>

    <details>
      <summary>{copy.settings.advanced}</summary>
      <button type="button" onclick={() => (confirming = true)}>{copy.settings.removeData}</button>
    </details>
  {/if}
</section>

{#if confirming}
  <Dialog title={copy.settings.removeDataTitle} onClose={closeConfirm}>
    <p>{copy.settings.removeDataIntro}</p>
    <ul>
      {#each copy.settings.removeDataItems as item (item)}
        <li>{item}</li>
      {/each}
    </ul>
    <p>{copy.settings.removeDataKeeps}</p>
    {#if removeError !== null}
      <p class="error" role="alert">{removeError}</p>
    {/if}
    {#snippet actions()}
      <button type="button" class="danger" disabled={removing} onclick={removeData}>
        {copy.settings.removeDataConfirm}
      </button>
      <button type="button" disabled={removing} onclick={closeConfirm}>{copy.common.cancel}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 40rem;
    padding: var(--space-5);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .top h1 {
    margin: 0;
  }
  fieldset {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-3) var(--space-4) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  legend {
    padding: 0 var(--space-1);
    font-weight: 600;
  }
  .choice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .sub {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 0 calc(var(--space-2) + 13px + var(--space-2));
  }
  .help {
    margin-top: calc(-1 * var(--space-1));
    color: var(--text-muted);
    font-size: var(--text-small);
  }
  input[type='text'] {
    min-width: 14rem;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text);
    font: inherit;
  }
  .path {
    color: var(--text-muted);
    word-break: break-all;
  }
  .mac-choice {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .links {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  summary {
    cursor: pointer;
    color: var(--text-muted);
  }
  details > button {
    margin-top: var(--space-2);
  }
  p,
  ul {
    margin: 0;
  }
  .error {
    color: var(--danger);
  }
</style>
