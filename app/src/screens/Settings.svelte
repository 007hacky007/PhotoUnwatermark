<script lang="ts">
  import { onMount } from 'svelte';
  import { appStatus, chooseFolder, getSettings, removeDownloadedData, setSettings } from '../lib/api';
  import DeviceChip from '../lib/components/DeviceChip.svelte';
  import Dialog from '../lib/components/Dialog.svelte';
  import { copy } from '../lib/copy';
  import type { Detail, Settings, UserError } from '../lib/types';
  import type { Wizard } from '../lib/wizard.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  let settings: Settings | null = $state(null);
  let platform = $state('');
  let error: string | null = $state(null);
  /** Which output choice is shown as picked; differs from `settings` while a folder is being chosen. */
  let outputKind: 'next' | 'folder' = $state('next');
  let folderName = $state('');
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

  function show(s: Settings) {
    settings = s;
    outputKind = 'nextToOriginals' in s.output ? 'next' : 'folder';
    if ('nextToOriginals' in s.output) folderName = s.output.nextToOriginals.folderName;
  }

  function loaded(result: PromiseSettledResult<Settings>) {
    if (result.status === 'fulfilled') show(result.value);
    else error = messageOf(result.reason, copy.settings.loadFailed);
  }

  /** Reads the settings again (a fix in the "why?" dialog may have changed them). */
  async function reload() {
    const [result] = await Promise.allSettled([getSettings()]);
    loaded(result);
  }

  // The platform comes first, so the device part never shows the wrong wording.
  onMount(async () => {
    const [system, result] = await Promise.allSettled([appStatus(), getSettings()]);
    platform = system.status === 'fulfilled' ? system.value.platform : '';
    loaded(result);
  });

  async function save(next: Settings) {
    error = null;
    try {
      await setSettings(next);
    } catch (e) {
      error = messageOf(e, copy.settings.saveFailed);
      if (settings !== null) show(settings);
      return;
    }
    const gpuChanged = settings !== null && settings.preferGpu !== next.preferGpu;
    show(next);
    if (gpuChanged) chipRefresh++;
  }

  /** The folder name kept when switching back from a fixed folder. */
  function savedFolderName(s: Settings): string {
    return 'nextToOriginals' in s.output ? s.output.nextToOriginals.folderName : folderName;
  }

  async function useNextToOriginals() {
    if (settings === null) return;
    outputKind = 'next';
    const name = folderName.trim() || savedFolderName(settings) || 'Without watermark';
    await save({ ...settings, output: { nextToOriginals: { folderName: name } } });
  }

  async function renameFolder() {
    if (settings === null) return;
    const name = folderName.trim();
    if (name === '') {
      folderName = savedFolderName(settings);
      return;
    }
    folderName = name;
    if ('nextToOriginals' in settings.output && settings.output.nextToOriginals.folderName === name) return;
    await save({ ...settings, output: { nextToOriginals: { folderName: name } } });
  }

  async function pickFolder() {
    if (settings === null) return;
    outputKind = 'folder';
    const path = await chooseFolder().catch(() => null);
    if (path === null) {
      // Cancelled: show the saved choice again.
      show(settings);
      return;
    }
    await save({ ...settings, output: { folder: { path } } });
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
            checked={settings.detail === d.value}
            aria-describedby="{id}-{d.value}"
            onchange={() => settings && save({ ...settings, detail: d.value })}
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
            checked={settings.preferGpu}
            onchange={() => settings && save({ ...settings, preferGpu: true })}
          />
          {copy.settings.useGraphicsCard}
        </label>
        <label class="choice">
          <input
            type="radio"
            name="{id}-device"
            checked={!settings.preferGpu}
            onchange={() => settings && save({ ...settings, preferGpu: false })}
          />
          {copy.settings.useProcessor}
        </label>
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
