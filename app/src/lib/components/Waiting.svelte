<script lang="ts">
  import { copy } from '../copy';

  type Props = {
    sentence: string;
    /** 0 to 1, or null when the time is unknown (the bar then moves without a value). */
    fraction: number | null;
    detail?: string;
    onCancel?: () => void;
    /** The cancel button's text; "Cancel" when left out. */
    cancelLabel?: string;
  };

  let { sentence, fraction, detail, onCancel, cancelLabel }: Props = $props();

  const id = $props.id();
  const percent = $derived(fraction === null ? null : Math.round(Math.min(1, Math.max(0, fraction)) * 100));
</script>

<div class="waiting">
  <p class="sentence" id="{id}-sentence">{sentence}</p>
  <div
    class="bar"
    role="progressbar"
    aria-labelledby="{id}-sentence"
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={percent ?? undefined}
    data-indeterminate={percent === null}
  >
    <div class="fill" style:width={percent === null ? undefined : `${percent}%`}></div>
  </div>
  {#if detail}
    <p class="detail">{detail}</p>
  {/if}
  {#if onCancel}
    <button type="button" onclick={onCancel}>{cancelLabel ?? copy.common.cancel}</button>
  {/if}
</div>

<style>
  .waiting {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    width: min(32rem, 100%);
    margin: 0 auto;
    padding: var(--space-6) var(--space-4);
    text-align: center;
  }
  .sentence {
    margin: 0;
    font-size: var(--text-large);
  }
  .detail {
    margin: 0;
    color: var(--text-muted);
  }
  .bar {
    position: relative;
    width: 100%;
    height: 8px;
    overflow: hidden;
    border-radius: 4px;
    background: var(--track);
  }
  .fill {
    height: 100%;
    border-radius: inherit;
    background: var(--accent);
    transition: width 0.2s linear;
  }
  .bar[data-indeterminate='true'] .fill {
    position: absolute;
    width: 30%;
    animation: slide 1.4s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      left: -30%;
    }
    to {
      left: 100%;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .fill {
      transition: none;
    }
    .bar[data-indeterminate='true'] .fill {
      left: 0;
      width: 100%;
      opacity: 0.5;
      animation: none;
    }
  }
</style>
