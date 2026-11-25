<script lang="ts">
  import ProgressLine from './lib/components/ProgressLine.svelte';
  import { copy } from './lib/copy';
  import { wizard as appWizard, type Wizard } from './lib/wizard.svelte';

  let { wizard = appWizard }: { wizard?: Wizard } = $props();
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
