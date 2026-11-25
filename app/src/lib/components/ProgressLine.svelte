<script lang="ts">
  import { copy } from '../copy';

  let { step }: { step: number } = $props();
</script>

<nav aria-label={copy.common.steps}>
  <ol>
    {#each copy.steps as label, i (label)}
      {@const n = i + 1}
      <li data-state={n < step ? 'done' : n === step ? 'current' : 'todo'} aria-current={n === step ? 'step' : undefined}>
        {#if i > 0}<span class="sep" aria-hidden="true">&gt;</span>{' '}{/if}<span class="num">{n}</span>
        {label}
      </li>
      {' '}
    {/each}
  </ol>
</nav>

<style>
  nav {
    padding: var(--space-2) var(--space-5);
    border-bottom: 1px solid var(--border);
  }
  ol {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-small);
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-muted);
  }
  .num {
    display: inline-grid;
    place-items: center;
    width: 1.5em;
    height: 1.5em;
    border-radius: 50%;
    border: 1px solid currentColor;
    font-size: 0.85em;
  }
  .sep {
    color: var(--text-faint);
  }
  li[data-state='done'] {
    color: var(--text);
  }
  li[data-state='done'] .num {
    border-color: var(--accent);
    color: var(--accent);
  }
  li[data-state='current'] {
    color: var(--text);
    font-weight: 600;
  }
  li[data-state='current'] .num {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
</style>
