<script lang="ts">
  import { untrack } from "svelte";

  type MascotStatus =
    | "idle"
    | "running"
    | "permission_required"
    | "completed"
    | "failed"
    | "waiting_for_input";
  type VisualStatus = MascotStatus | "awake";
  type TransitionPhase =
    | "steady"
    | "waking"
    | "shifting"
    | "arriving"
    | "settling"
    | "falling-asleep";

  let {
    status = "idle",
    size = 30,
    awake = false,
  }: { status?: MascotStatus; size?: number; awake?: boolean } = $props();

  const targetStatus = $derived<VisualStatus>(status === "idle" && awake ? "awake" : status);
  let displayStatus = $state<VisualStatus>("idle");
  let phase = $state<TransitionPhase>("steady");

  $effect(() => {
    const next = targetStatus;
    const current = untrack(() => displayStatus);
    if (next === current && untrack(() => phase) === "steady") return;

    const timers: ReturnType<typeof setTimeout>[] = [];
    const later = (delay: number, action: () => void) => {
      timers.push(setTimeout(action, delay));
    };

    if (current === "idle" && next !== "idle") {
      phase = "waking";
      later(260, () => {
        displayStatus = next;
        phase = "arriving";
      });
      later(520, () => (phase = "steady"));
    } else if (next === "idle") {
      phase = "settling";
      later(240, () => {
        displayStatus = "idle";
        phase = "falling-asleep";
      });
      later(560, () => (phase = "steady"));
    } else {
      phase = "shifting";
      later(170, () => {
        displayStatus = next;
        phase = "arriving";
      });
      later(430, () => (phase = "steady"));
    }

    return () => timers.forEach(clearTimeout);
  });
</script>

<span
  class="mascot status-{displayStatus} phase-{phase}"
  style:--mascot-size={`${size}px`}
  aria-hidden="true"
>
  <svg viewBox="0 0 32 32" shape-rendering="crispEdges">
    <g class="dino">
      <path
        class="body"
        d="M4 16h3v-2h3V9h3V6h11v2h3v8h-9v2h4v4h-5v4h-4v-5h-3v5H7v-5H5v-2H3v-4h1z"
      />
      <path class="belly" d="M10 15h3v2h5v3h-8z" />
      <rect class="snout" x="23" y="13" width="4" height="2" />

      {#if displayStatus === "idle"}
        <rect class="face" x="21" y="10" width="4" height="1" />
      {:else if displayStatus === "failed"}
        <path class="face" d="M21 9h1v1h1V9h1v3h-1v-1h-1v1h-1z" />
        <path class="mouth" d="M22 14h1v-1h3v1h-1v1h-3z" />
      {:else}
        <rect class="eye-light" x="21" y="9" width="3" height="3" />
        <rect class="face" x="23" y="9" width="1" height="2" />
        {#if displayStatus === "completed"}
          <path class="mouth" d="M22 13h1v1h3v-1h1v2h-1v1h-3v-1h-1z" />
        {/if}
      {/if}

      <g class="feet">
        <rect x="7" y="25" width="5" height="2" />
        <rect x="13" y="25" width="5" height="2" />
      </g>
    </g>

    {#if displayStatus === "idle"}
      <g class="sleep-pixels">
        <path d="M24 3h5v1h-3v1h3v1h-5V5h2V4h-2z" />
        <path d="M27 0h3v1h-1v1h1v1h-3V2h1V1h-1z" />
      </g>
    {:else if displayStatus === "running"}
      <g class="motion-pixels">
        <rect x="1" y="22" width="3" height="2" />
        <rect x="3" y="26" width="2" height="1" />
      </g>
    {:else if displayStatus === "completed"}
      <path class="state-pixels success" d="M26 2h2v2h2v2h-2v2h-2V6h-2V4h2z" />
    {:else if displayStatus === "failed"}
      <path class="state-pixels failure" d="M26 2h2v4h-2zm0 5h2v2h-2z" />
    {:else if displayStatus === "permission_required"}
      <path class="state-pixels attention" d="M26 1h3v6h-3zm0 7h3v3h-3z" />
    {:else if displayStatus === "waiting_for_input"}
      <path class="state-pixels waiting" d="M25 2h4v1h1v3h-1v1h-1v2h-2V6h2V4h-3zm1 8h2v2h-2z" />
    {/if}
  </svg>
</span>

<style>
  .mascot {
    --mascot-color: #8b9490;
    position: relative;
    width: var(--mascot-size);
    height: var(--mascot-size);
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
    color: #263d35;
  }

  svg { width: 100%; height: 100%; overflow: visible; }
  .body { fill: var(--mascot-color); transition: fill 280ms ease; }
  .belly { fill: rgba(255, 255, 255, 0.34); transition: fill 280ms ease; }
  .snout { fill: rgba(24, 48, 39, 0.16); }
  .face,
  .mouth,
  .feet { fill: currentColor; }
  .eye-light { fill: #f8fbf9; }
  .sleep-pixels { fill: #828b87; animation: sleep-float 2.4s steps(3, end) infinite; }
  .motion-pixels { fill: #6695bd; animation: dust 600ms steps(2, end) infinite; }
  .state-pixels { transform-origin: center; }
  .success { fill: #5b9d79; animation: sparkle 1.25s steps(2, end) infinite; }
  .failure { fill: #bd625f; animation: alert-pop 1.3s steps(2, end) infinite; }
  .attention { fill: #c88336; animation: alert-pop 900ms steps(2, end) infinite; }
  .waiting { fill: #6386ad; animation: question-bob 1.5s steps(2, end) infinite; }

  .status-idle { --mascot-color: #8b9490; }
  .status-idle .dino { animation: breathe 2.4s steps(2, end) infinite; }
  .status-awake { --mascot-color: #63a57d; }
  .status-awake .dino { animation: look-around 2.8s steps(2, end) infinite; }
  .status-running { --mascot-color: #5f91b9; }
  .status-running .dino { animation: run 520ms steps(2, end) infinite; }
  .status-running .feet { animation: feet-run 260ms steps(2, end) infinite; transform-origin: center; }
  .status-permission_required { --mascot-color: #cb8b45; }
  .status-permission_required .dino { animation: listen 1s steps(2, end) infinite; }
  .status-completed { --mascot-color: #65a480; }
  .status-completed .dino { animation: celebrate 1.5s steps(2, end) infinite; }
  .status-failed { --mascot-color: #bd6965; }
  .status-failed .dino { animation: shake 1.4s steps(2, end) infinite; }
  .status-waiting_for_input { --mascot-color: #758ead; }
  .status-waiting_for_input .dino { animation: listen 1.7s steps(2, end) infinite; }

  .phase-waking { --mascot-color: #63a57d; }
  .phase-waking .dino { animation: wake-up 260ms steps(3, end) both; }
  .phase-waking .sleep-pixels { animation: sleep-away 200ms steps(2, end) both; }
  .phase-shifting .dino { animation: anticipate 170ms ease both; }
  .phase-arriving .dino { animation: arrive 260ms cubic-bezier(0.22, 1, 0.36, 1) both; }
  .phase-settling .dino { animation: settle 240ms ease both; }
  .phase-falling-asleep .dino { animation: doze-off 320ms steps(3, end) both; }
  .phase-falling-asleep .sleep-pixels { animation: sleep-in 320ms steps(3, end) both; }

  @keyframes breathe { 50% { transform: translateY(1px); } }
  @keyframes sleep-float { 50% { opacity: 0.45; transform: translate(1px, -1px); } }
  @keyframes look-around { 40% { transform: translateX(1px); } 70% { transform: translateX(-1px); } }
  @keyframes run { 50% { transform: translateY(-2px); } }
  @keyframes feet-run { 50% { transform: translateX(2px); } }
  @keyframes dust { 50% { opacity: 0.25; transform: translateX(-2px); } }
  @keyframes listen { 50% { transform: rotate(-2deg) translateY(-1px); } }
  @keyframes celebrate { 35% { transform: translateY(-3px) rotate(-2deg); } }
  @keyframes shake { 25% { transform: translateX(-1px); } 50% { transform: translateX(1px); } }
  @keyframes sparkle { 50% { opacity: 0.35; transform: scale(0.7); } }
  @keyframes alert-pop { 50% { transform: translateY(-1px) scale(1.12); } }
  @keyframes question-bob { 50% { transform: translateY(-2px); } }
  @keyframes wake-up { 0% { transform: translateY(1px) scaleY(0.96); } 70% { transform: translateY(-2px) scaleY(1.04); } 100% { transform: none; } }
  @keyframes sleep-away { to { opacity: 0; transform: translate(2px, -3px); } }
  @keyframes anticipate { 50% { transform: translateY(1px) scale(0.96); } }
  @keyframes arrive { 0% { transform: translateY(2px) scale(0.94); } 65% { transform: translateY(-1px) scale(1.03); } 100% { transform: none; } }
  @keyframes settle { 60% { transform: translateY(1px) scaleY(0.96); } }
  @keyframes doze-off { from { transform: translateY(-1px); } to { transform: translateY(1px) scaleY(0.97); } }
  @keyframes sleep-in { from { opacity: 0; transform: translate(-2px, 2px); } to { opacity: 1; transform: none; } }

  @media (prefers-color-scheme: dark) {
    .mascot { color: #20322c; }
    .belly { fill: rgba(255, 255, 255, 0.22); }
    .eye-light { fill: #eaf2ee; }
  }

  @media (prefers-reduced-motion: reduce) {
    .mascot *,
    .mascot { animation: none !important; transition: none !important; }
  }
</style>
