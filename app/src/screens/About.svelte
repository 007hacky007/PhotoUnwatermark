<script lang="ts">
  import { onMount } from 'svelte';
  import { appStatus, openReleasesPage, RELEASES_URL } from '../lib/api';
  import { copy } from '../lib/copy';
  import type { Wizard } from '../lib/wizard.svelte';

  let { wizard }: { wizard: Wizard } = $props();

  let version: string | null = $state(null);

  onMount(() => {
    appStatus()
      .then((s) => (version = s.version))
      .catch(() => {});
  });

  // The page opens in the browser, never inside the app window.
  function openReleases(e: MouseEvent) {
    e.preventDefault();
    openReleasesPage().catch(() => {});
  }
</script>

<section class="about">
  <div class="top">
    <button type="button" onclick={() => wizard.back()}>{copy.common.back}</button>
    <h1>{copy.about.title}</h1>
  </div>

  {#if version !== null}
    <p class="muted">{copy.about.version(version)}</p>
  {/if}
  <p>{copy.about.responsibleUse}</p>

  <h2>{copy.about.credits}</h2>
  <ul>
    <li>{copy.about.finder}</li>
    <li>
      <p>{copy.about.remover}</p>
      <p class="muted">{copy.about.removerWeights}</p>
    </li>
    <li>{copy.about.faces}</li>
    <li>{copy.about.app}</li>
  </ul>

  <p>
    {copy.about.newVersions}
    <a href={RELEASES_URL} onclick={openReleases}>{copy.about.releases}</a>.
  </p>
</section>

<style>
  .about {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
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
  h2 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-large);
    font-weight: 600;
  }
  p,
  ul {
    margin: 0;
  }
  ul {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-left: var(--space-5);
  }
  .muted {
    color: var(--text-muted);
  }
  a {
    color: var(--accent);
  }
</style>
