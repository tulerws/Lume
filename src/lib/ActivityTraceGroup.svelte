<script lang="ts">
  import { slide } from "svelte/transition";
  import type { SessionActivity } from "$lib/domain";
  import type { Language } from "$lib/i18n";
  import { activityCategory, activityDisplayTitle, activityGroupSummary, activityPreview } from "$lib/activityPresentation";

  let { activities, language = "en", active = false } = $props<{
    activities: SessionActivity[];
    language?: Language;
    active?: boolean;
  }>();

  const summary = $derived(activityGroupSummary(activities, language));
  let visibleActivityLimit = $state(40);
  const hiddenActivityCount = $derived(Math.max(0, activities.length - visibleActivityLimit));
  const visibleActivities = $derived(
    hiddenActivityCount > 0 ? activities.slice(-visibleActivityLimit) : activities,
  );
  let expanded = $state(true);
  let initialized = false;
  let wasActive = false;

  $effect(() => {
    const isActive = active || activities.some(
      (activity: SessionActivity) => activity.status === "running",
    );
    if (!initialized) {
      expanded = isActive;
      initialized = true;
    } else if (isActive) {
      expanded = true;
    } else if (wasActive) {
      expanded = false;
    }
    wasActive = isActive;
  });

  function tr(english: string, portuguese: string) {
    return language === "pt-BR" ? portuguese : english;
  }
</script>

<section class:open={expanded} class="activity-cluster">
  <button
    class="cluster-summary"
    type="button"
    aria-expanded={expanded}
    onclick={() => (expanded = !expanded)}
  >
    <span class="cluster-mark">
      <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 6h12M4 10h8M4 14h10" /></svg>
    </span>
    <strong>{summary}</strong>
    <small>{activities.length}</small>
    <svg class="cluster-chevron" viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4" /></svg>
  </button>
  {#if expanded}
    <div class="activity-list" transition:slide={{ duration: 180 }}>
      {#if hiddenActivityCount > 0}
        <button class="load-earlier-activities" type="button" onclick={() => (visibleActivityLimit += 40)}>
          {tr(`Show ${Math.min(40, hiddenActivityCount)} earlier events`, `Mostrar ${Math.min(40, hiddenActivityCount)} eventos anteriores`)}
        </button>
      {/if}
      {#each visibleActivities as activity (activity.id)}
        {@const category = activityCategory(activity)}
        {@const preview = activityPreview(activity)}
        <details class:failed={activity.status === "failed"} class:running={activity.status === "running"} class="activity-row">
          <summary>
            <i class="activity-status" aria-label={activity.status}></i>
            <span class="activity-icon" data-category={category}>
              {#if category === "edit"}
                <svg viewBox="0 0 20 20"><path d="m5 14 1-4 7-7 3 3-7 7zM12 4l3 3" /></svg>
              {:else if category === "read"}
                <svg viewBox="0 0 20 20"><path d="M3.5 5.5h5l1.5 2h6.5v8h-13zM7 11h6M7 14h4" /></svg>
              {:else if category === "search"}
                <svg viewBox="0 0 20 20"><circle cx="8.5" cy="8.5" r="4.5" /><path d="m12 12 4 4" /></svg>
              {:else if category === "test"}
                <svg viewBox="0 0 20 20"><path d="m4 10 3.5 3.5L16 5" /></svg>
              {:else if category === "command"}
                <svg viewBox="0 0 20 20"><path d="m4 6 4 4-4 4M10 14h6" /></svg>
              {:else}
                <svg viewBox="0 0 20 20"><path d="M10 3v3M10 14v3M3 10h3M14 10h3M5 5l2 2M13 13l2 2M15 5l-2 2M7 13l-2 2" /></svg>
              {/if}
            </span>
            <span class="activity-copy">
              <strong>{activityDisplayTitle(activity, language)}</strong>
              {#if preview}<code title={preview}>{preview}</code>{/if}
            </span>
            {#if activity.detail}<svg class="row-chevron" viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4" /></svg>{/if}
          </summary>
          {#if activity.detail}<pre>{activity.detail}</pre>{/if}
        </details>
      {/each}
    </div>
  {/if}
</section>

<style>
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.55; stroke-linecap: round; stroke-linejoin: round; }
  .activity-cluster { box-sizing: border-box; width: 100%; min-width: 0; max-width: 100%; overflow: hidden; border: 1px solid rgba(65, 94, 80, .18); border-radius: 9px; background: #f6f9f7; }
  .cluster-summary { width: 100%; min-height: 34px; padding: 5px 7px; display: flex; align-items: center; gap: 7px; border: 0; color: #65776e; background: transparent; cursor: pointer; text-align: left; }
  .activity-row > summary::-webkit-details-marker { display: none; }
  .cluster-mark { width: 22px; height: 22px; display: grid; place-items: center; flex: 0 0 auto; border-radius: 6px; color: #428066; background: rgba(61, 143, 101, .08); }
  .cluster-summary > strong { min-width: 0; flex: 1; overflow: hidden; font: 730 var(--chat-small-font-size, 9px)/1.35 Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .cluster-summary > small { min-width: 17px; height: 17px; padding: 0 4px; display: grid; place-items: center; border-radius: 9px; color: #778980; background: rgba(76, 105, 91, .07); font: 700 var(--chat-tiny-font-size, 7px) Inter, sans-serif; }
  .cluster-chevron, .row-chevron { flex: 0 0 auto; color: #89968f; transition: transform 140ms ease; }
  .activity-cluster.open .cluster-chevron, .activity-row[open] > summary .row-chevron { transform: rotate(180deg); }
  .activity-list { padding: 1px 6px 6px 15px; display: grid; }
  .load-earlier-activities { min-height: 27px; margin: 2px 4px 3px 0; border: 0; border-bottom: 1px solid rgba(75, 114, 94, .1); color: #778a80; background: transparent; font: 700 var(--chat-tiny-font-size, 7px) Inter, sans-serif; cursor: pointer; text-align: left; }
  .load-earlier-activities:hover { color: #3f7f61; }
  .activity-row { position: relative; min-width: 0; border-left: 1px solid rgba(65, 105, 85, .2); }
  .activity-row > summary { min-width: 0; min-height: 34px; padding: 4px 2px 4px 9px; display: flex; align-items: center; gap: 7px; color: #61736a; cursor: pointer; list-style: none; }
  .activity-icon { width: 21px; height: 21px; display: grid; place-items: center; flex: 0 0 auto; border-radius: 6px; color: #658478; background: rgba(79, 117, 98, .055); }
  .activity-icon[data-category="edit"] { color: #37845c; background: rgba(55, 132, 92, .08); }
  .activity-icon[data-category="search"] { color: #557fa2; background: rgba(85, 127, 162, .08); }
  .activity-icon[data-category="test"] { color: #5a8a70; background: rgba(74, 142, 105, .08); }
  .activity-icon[data-category="command"] { color: #687b94; background: rgba(85, 110, 143, .075); }
  .activity-copy { min-width: 0; flex: 1; display: grid; gap: 2px; }
  .activity-copy strong { overflow: hidden; color: #52665c; font: 700 var(--chat-small-font-size, 9px)/1.25 Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .activity-copy code { overflow: hidden; color: #89958f; font: var(--chat-tiny-font-size, 7px)/1.3 "SFMono-Regular", Consolas, monospace; text-overflow: ellipsis; white-space: nowrap; }
  .activity-status { position: absolute; z-index: 1; top: 14px; left: -3.5px; width: 7px; height: 7px; border-radius: 50%; background: #55a778; box-shadow: 0 0 0 2px #f8fbf9; }
  .activity-row.running .activity-status { will-change: transform, opacity; background: #4e91bf; animation: activity-pulse 1s ease-in-out infinite; }
  .activity-row.failed .activity-status { background: #b85d59; }
  .activity-row pre { min-width: 0; max-width: calc(100% - 18px); max-height: 180px; margin: 0 0 7px 18px; padding: 6px 7px; overflow: auto; border-radius: 6px; color: #4f6258; background: #eaf0ed; font: var(--chat-tiny-font-size, 7px)/1.5 "SFMono-Regular", Consolas, monospace; overflow-wrap: anywhere; white-space: pre-wrap; word-break: break-word; }
  :global(.terminal-window.dark) .activity-cluster { border-color: rgba(205, 222, 213, .075); background: rgba(218, 234, 226, .018); }
  :global(.terminal-window.dark) .cluster-summary { color: #a1b3aa; }
  :global(.terminal-window.dark) .cluster-mark { color: #8bc5a8; background: rgba(91, 177, 137, .075); }
  :global(.terminal-window.dark) .cluster-summary > small { color: #8fa198; background: rgba(205, 222, 213, .055); }
  :global(.terminal-window.dark) .activity-row { border-color: rgba(177, 207, 191, .1); }
  :global(.terminal-window.dark) .activity-status { box-shadow: 0 0 0 2px #141d19; }
  :global(.terminal-window.dark) .activity-copy strong { color: #bccdc4; }
  :global(.terminal-window.dark) .activity-copy code { color: #7f9188; }
  :global(.terminal-window.dark) .activity-icon { color: #94aa9f; background: rgba(205, 222, 213, .04); }
  :global(.terminal-window.dark) .activity-row pre { color: #adbbb4; background: rgba(4, 12, 8, .18); }
  :global(.terminal-window.dark) .load-earlier-activities { color: #8fa198; border-color: rgba(177, 207, 191, .08); }
  :global(.terminal-window.dark) .load-earlier-activities:hover { color: #8bc5a8; }
  @keyframes activity-pulse { 50% { opacity: .4; transform: scale(.75); } }
</style>
