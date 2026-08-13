<script lang="ts">
  import type { WorkflowRole } from "$lib/domain";

  let { role } = $props<{ role: WorkflowRole }>();
</script>

<svg
  class="workflow-role-icon role-{role}"
  viewBox="0 0 24 24"
  fill="none"
  aria-hidden="true"
>
  {#if role === "planner"}
    <path d="M7 4h10.5A2.5 2.5 0 0 1 20 6.5V20H7a3 3 0 1 1 0-6h1V6a2 2 0 0 0-4 0v11" />
    <path d="m10.5 15.5 1-3 6.1-7 2.4 2.1-6.1 7-3.4.9ZM16.8 6.4l2.4 2.1M11 18h5" />
  {:else if role === "implementer"}
    <path d="m7 5-4.5 7L7 19M17 5l4.5 7-4.5 7M14.8 3 9.2 21" />
  {:else if role === "reviewer"}
    <circle cx="10.5" cy="10.5" r="6.5" />
    <path d="m15.5 15.5 5 5M7.5 10.5l2 2 4-4" />
  {:else if role === "tester"}
    <path d="M9 3h6M10 3v5l-5 9a2 2 0 0 0 1.8 3h10.4a2 2 0 0 0 1.8-3l-5-9V3" />
    <path d="M7.5 15h9" />
    <circle cx="10" cy="12" r=".7" />
    <circle cx="14" cy="17" r=".7" />
  {:else if role === "researcher"}
    <circle cx="12" cy="12" r="8.5" />
    <path d="m16.8 7.2-2.8 6.8-6.8 2.8L10 10l6.8-2.8Z" />
    <circle cx="12" cy="12" r="1.35" />
  {:else}
    <path d="M4 6h7M15 6h5M4 12h3M11 12h9M4 18h10M18 18h2" />
    <circle cx="13" cy="6" r="2" />
    <circle cx="9" cy="12" r="2" />
    <circle cx="16" cy="18" r="2" />
  {/if}
</svg>

<style>
  .workflow-role-icon {
    width: 68%;
    height: 68%;
    display: block;
    overflow: visible;
    color: inherit;
    stroke: currentColor;
    stroke-width: 1.65;
    stroke-linecap: round;
    stroke-linejoin: round;
    transform-box: fill-box;
    transform-origin: center;
    transition: transform 180ms cubic-bezier(.2,.8,.2,1), filter 180ms ease;
  }

  @media (prefers-reduced-motion: reduce) {
    .workflow-role-icon { animation: none !important; transition: none; }
  }
</style>
