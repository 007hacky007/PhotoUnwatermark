<script lang="ts">
  import { onMount } from 'svelte';
  import { appStatus } from './lib/api';
  import ProgressLine from './lib/components/ProgressLine.svelte';
  import { copy } from './lib/copy';
  import { wizard as appWizard, type Wizard } from './lib/wizard.svelte';
  import AddPhotos from './screens/AddPhotos.svelte';
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
      <!-- Placeholder until the watermark editor; it reads wizard.outcome. -->
      <section class="page">
        <h1>{copy.steps[1]}</h1>
      </section>
    {/if}
    {#if wizard.screen === 'settings'}
      <section class="page">
        <h1>{copy.settings.title}</h1>
        <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
      </section>
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
  .page {
    padding: var(--space-5);
  }
</style>
