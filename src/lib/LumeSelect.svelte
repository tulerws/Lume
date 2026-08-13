<script lang="ts">
  import { onMount, tick } from "svelte";

  export type LumeSelectOption = {
    value: string;
    label: string;
    description?: string;
  };

  let {
    value,
    options,
    ariaLabel,
    onValueChange,
    minWidth = 120,
  }: {
    value: string;
    options: LumeSelectOption[];
    ariaLabel: string;
    onValueChange: (value: string) => void;
    minWidth?: number;
  } = $props();

  let root = $state<HTMLDivElement | null>(null);
  let trigger = $state<HTMLButtonElement | null>(null);
  let open = $state(false);
  let menuLeft = $state(0);
  let menuTop = $state(0);
  let menuWidth = $state(120);
  let menuMaxHeight = $state(220);
  let activeIndex = $state(0);
  const selected = $derived(options.find((option) => option.value === value) ?? options[0]);

  function placeMenu() {
    if (!trigger) return;
    const bounds = trigger.getBoundingClientRect();
    const estimatedHeight = Math.min(230, options.length * 43 + 10);
    const below = window.innerHeight - bounds.bottom - 8;
    const above = bounds.top - 8;
    const placeAbove = below < Math.min(150, estimatedHeight) && above > below;
    menuWidth = Math.min(Math.max(bounds.width, minWidth), window.innerWidth - 16);
    menuLeft = Math.max(8, Math.min(window.innerWidth - menuWidth - 8, bounds.right - menuWidth));
    menuMaxHeight = Math.max(92, Math.min(230, placeAbove ? above - 5 : below - 5));
    menuTop = placeAbove
      ? Math.max(8, bounds.top - Math.min(estimatedHeight, menuMaxHeight) - 5)
      : bounds.bottom + 5;
  }

  async function toggle() {
    open = !open;
    if (!open) return;
    activeIndex = Math.max(0, options.findIndex((option) => option.value === value));
    await tick();
    placeMenu();
  }

  function choose(option: LumeSelectOption) {
    open = false;
    if (option.value !== value) onValueChange(option.value);
    trigger?.focus();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      open = false;
      trigger?.focus();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!open) {
        void toggle();
        return;
      }
      const direction = event.key === "ArrowDown" ? 1 : -1;
      activeIndex = (activeIndex + direction + options.length) % options.length;
      return;
    }
    if ((event.key === "Enter" || event.key === " ") && open) {
      event.preventDefault();
      choose(options[activeIndex]);
    }
  }

  onMount(() => {
    const closeOutside = (event: PointerEvent) => {
      if (open && root && !root.contains(event.target as Node)) open = false;
    };
    const reposition = () => open && placeMenu();
    document.addEventListener("pointerdown", closeOutside);
    window.addEventListener("resize", reposition);
    document.addEventListener("scroll", reposition, true);
    return () => {
      document.removeEventListener("pointerdown", closeOutside);
      window.removeEventListener("resize", reposition);
      document.removeEventListener("scroll", reposition, true);
    };
  });
</script>

<div class="lume-select" bind:this={root} style:min-width="{minWidth}px">
  <button
    bind:this={trigger}
    class:open
    class="lume-select-trigger"
    type="button"
    aria-label={ariaLabel}
    aria-haspopup="listbox"
    aria-expanded={open}
    onkeydown={handleKeydown}
    onclick={() => void toggle()}
  >
    <span>{selected?.label ?? "—"}</span>
    <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4"></path></svg>
  </button>

  {#if open}
    <div
      class="lume-select-menu"
      role="listbox"
      tabindex="-1"
      aria-label={ariaLabel}
      style:left="{menuLeft}px"
      style:top="{menuTop}px"
      style:width="{menuWidth}px"
      style:max-height="{menuMaxHeight}px"
      onkeydown={handleKeydown}
    >
      {#each options as option, index (option.value)}
        <button
          class:active={option.value === value}
          class:focused={index === activeIndex}
          type="button"
          role="option"
          aria-selected={option.value === value}
          onpointerenter={() => (activeIndex = index)}
          onclick={() => choose(option)}
        >
          <span><strong>{option.label}</strong>{#if option.description}<small>{option.description}</small>{/if}</span>
          {#if option.value === value}<svg viewBox="0 0 20 20" aria-hidden="true"><path d="m5 10 3 3 7-7"></path></svg>{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .lume-select { position: relative; min-width: 0; flex: 0 1 auto; }
  .lume-select-trigger { width: 100%; min-height: 30px; padding: 0 8px 0 10px; display: flex; align-items: center; gap: 7px; border: 1px solid rgba(79, 111, 95, .14); border-radius: 9px; color: #4a5d53; background: linear-gradient(145deg, rgba(255,255,255,.72), rgba(72,126,98,.035)); box-shadow: 0 1px 2px rgba(31,54,42,.035); cursor: pointer; text-align: left; transition: border-color 140ms ease, background 140ms ease, box-shadow 140ms ease; }
  .lume-select-trigger:hover,
  .lume-select-trigger.open { border-color: rgba(55, 143, 97, .3); background: rgba(255,255,255,.82); box-shadow: 0 3px 10px rgba(31,54,42,.07); }
  .lume-select-trigger > span { min-width: 0; flex: 1; overflow: hidden; font: 700 9px Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .lume-select-trigger > svg { width: 13px; height: 13px; flex: 0 0 auto; fill: none; stroke: #71857a; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.5; transition: transform 140ms ease; }
  .lume-select-trigger.open > svg { transform: rotate(180deg); }
  .lume-select-trigger:focus-visible { outline: 2px solid rgba(74, 148, 108, .3); outline-offset: 2px; }
  .lume-select-menu { position: fixed; z-index: 260; padding: 5px; display: grid; gap: 2px; overflow-y: auto; isolation: isolate; contain: paint; overscroll-behavior: contain; border: 1px solid rgba(73,108,90,.16); border-radius: 11px; background: #f8fbf9; background-clip: padding-box; box-shadow: 0 16px 38px rgba(20,42,31,.22); }
  .lume-select-menu > button { width: 100%; min-height: 34px; padding: 5px 7px; display: flex; align-items: center; gap: 6px; border: 0; border-radius: 8px; color: #53665d; background: transparent; cursor: pointer; text-align: left; }
  .lume-select-menu > button:hover,
  .lume-select-menu > button.focused { background: rgba(55,145,99,.055); }
  .lume-select-menu > button.active { color: #347653; background: rgba(55,145,99,.085); }
  .lume-select-menu > button > span { min-width: 0; flex: 1; display: grid; gap: 1px; }
  .lume-select-menu strong { overflow: hidden; font: 750 8px Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .lume-select-menu small { overflow: hidden; color: #85938c; font: 7px Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .lume-select-menu svg { width: 13px; height: 13px; fill: none; stroke: #3d9264; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.7; }
  :global(.overlay-shell.dark) .lume-select-trigger { color: #c6d4cd; border-color: rgba(205,222,213,.11); background: linear-gradient(145deg, rgba(222,235,228,.05), rgba(64,151,106,.025)); box-shadow: none; }
  :global(.overlay-shell.dark) .lume-select-trigger:hover,
  :global(.overlay-shell.dark) .lume-select-trigger.open { border-color: rgba(86,183,135,.25); background: rgba(222,235,228,.075); }
  :global(.overlay-shell.dark) .lume-select-trigger > svg { stroke: #8ca197; }
  :global(.overlay-shell.dark) .lume-select-menu { color: #c3d1ca; border-color: rgba(205,222,213,.12); background: #18221d; box-shadow: 0 18px 42px rgba(0,0,0,.42); }
  :global(.overlay-shell.dark) .lume-select-menu > button { color: #b8c9c0; }
  :global(.overlay-shell.dark) .lume-select-menu > button:hover,
  :global(.overlay-shell.dark) .lume-select-menu > button.focused { background: rgba(76,171,122,.06); }
  :global(.overlay-shell.dark) .lume-select-menu > button.active { color: #91d2b1; background: rgba(76,171,122,.1); }
  :global(.overlay-shell.dark) .lume-select-menu small { color: #81938a; }
</style>
