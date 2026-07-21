<script lang="ts">
  import OfflineIcon from "@iconify/svelte/dist/OfflineIcon.svelte";
  import brave from "@iconify-icons/logos/brave";
  import chrome from "@iconify-icons/logos/chrome";
  import edge from "@iconify-icons/logos/microsoft-edge";
  import openai from "@iconify-icons/logos/openai-icon";
  import vscode from "@iconify-icons/logos/visual-studio-code";
  import { siClaude, siGooglegemini } from "simple-icons";

  type Brand = "codex" | "claude" | "gemini" | "vscode" | "browsers" | "unknown";

  let { name, size = 18 }: { name: Brand; size?: number } = $props();
</script>

<span class="brand-icon" class:browsers={name === "browsers"} style:--brand-size={`${size}px`}>
  {#if name === "codex"}
    <OfflineIcon icon={openai} />
  {:else if name === "claude"}
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d={siClaude.path} fill="#d97757" />
    </svg>
  {:else if name === "gemini"}
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <defs>
        <linearGradient id="lume-gemini" x1="3" y1="21" x2="21" y2="3" gradientUnits="userSpaceOnUse">
          <stop stop-color="#4e82ee" />
          <stop offset="0.48" stop-color="#8e75d5" />
          <stop offset="1" stop-color="#d96570" />
        </linearGradient>
      </defs>
      <path d={siGooglegemini.path} fill="url(#lume-gemini)" />
    </svg>
  {:else if name === "vscode"}
    <OfflineIcon icon={vscode} />
  {:else if name === "browsers"}
    <i class="browser-icon chrome"><OfflineIcon icon={chrome} /></i>
    <i class="browser-icon edge"><OfflineIcon icon={edge} /></i>
    <i class="browser-icon brave"><OfflineIcon icon={brave} /></i>
  {:else}
    <svg class="unknown" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3v18M3 12h18M5.6 5.6l12.8 12.8M18.4 5.6 5.6 18.4" />
    </svg>
  {/if}
</span>

<style>
  .brand-icon {
    position: relative;
    width: var(--brand-size);
    height: var(--brand-size);
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
  }

  .brand-icon :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }

  .unknown {
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-width: 1.5;
  }

  .browsers { width: calc(var(--brand-size) * 1.28); }
  .browser-icon { position: absolute; width: 64%; height: 64%; display: grid; filter: drop-shadow(0 1px 1px rgba(25, 37, 32, 0.18)); }
  .browser-icon.chrome { left: 0; top: 0; }
  .browser-icon.edge { right: 0; top: 10%; }
  .browser-icon.brave { left: 26%; bottom: 0; }
</style>
