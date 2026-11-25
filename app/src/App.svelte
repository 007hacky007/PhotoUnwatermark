<script lang="ts">
  import { onMount } from 'svelte';
  import { appStatus } from './lib/api';
  import ProgressLine from './lib/components/ProgressLine.svelte';
  import { copy } from './lib/copy';
  import { wizard as appWizard, type Wizard } from './lib/wizard.svelte';
  import About from './screens/About.svelte';
  import AddPhotos from './screens/AddPhotos.svelte';
  import CheckAll from './screens/CheckAll.svelte';
  import CheckWatermark from './screens/CheckWatermark.svelte';
  import Done from './screens/Done.svelte';
  import Remove from './screens/Remove.svelte';
  import SavedWatermarks from './screens/SavedWatermarks.svelte';
  import Settings from './screens/Settings.svelte';
  import Welcome from './screens/Welcome.svelte';

  let { wizard = appWizard }: { wizard?: Wizard } = $props();

  // Start-up routing: with the models in place the app opens on Add photos, without them on
  // the welcome screen. The welcome screen waits for the answer so it never flashes.
  let checked = $state(false);
  onMount(async () => {
    if (wizard.screen !== 'welcome') {
      checked = true;
      return;
    }
    try {
      if ((await appStatus()).modelsReady) wizard.go('add');
    } catch {
      // Cannot tell: the welcome screen is the safe place, its download resumes or finishes at once.
    }
    checked = true;
  });
</script>

<div class="app">
  <header>
    <span class="name">{copy.appName}</span>
    {#if !wizard.busy && wizard.screen !== 'settings'}
      <button type="button" class="quiet" onclick={() => wizard.go('settings')}>{copy.common.settings}</button>
    {/if}
  </header>

  {#if wizard.step !== null}
    <ProgressLine step={wizard.step} />
  {/if}

  <main>
    {#if wizard.screen === 'welcome' && checked}
      <Welcome {wizard} />
    {/if}
    {#if wizard.screen === 'add'}
      <AddPhotos {wizard} />
    {/if}
    {#if wizard.screen === 'check'}
      <CheckWatermark {wizard} />
    {/if}
    {#if wizard.screen === 'checkAll'}
      <CheckAll {wizard} />
    {/if}
    {#if wizard.screen === 'remove'}
      <Remove {wizard} />
    {/if}
    {#if wizard.screen === 'done'}
      <Done {wizard} />
    {/if}
    {#if wizard.screen === 'settings'}
      <Settings {wizard} />
    {/if}
    {#if wizard.screen === 'saved'}
      <SavedWatermarks {wizard} />
    {/if}
    {#if wizard.screen === 'about'}
      <About {wizard} />
    {/if}
  </main>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 48px;
    padding: var(--space-2) var(--space-5);
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .name {
    font-weight: 600;
  }
  main {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
</style>
