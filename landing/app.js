const header = document.querySelector("[data-header]");
const revealItems = [...document.querySelectorAll(".reveal")];
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

function updateHeader() {
  header?.classList.toggle("scrolled", window.scrollY > 18);
}

updateHeader();
window.addEventListener("scroll", updateHeader, { passive: true });

if (reducedMotion || !("IntersectionObserver" in window)) {
  revealItems.forEach((item) => item.classList.add("visible"));
} else {
  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        entry.target.classList.add("visible");
        observer.unobserve(entry.target);
      }
    },
    { threshold: 0.14, rootMargin: "0px 0px -7%" },
  );
  revealItems.forEach((item, index) => {
    item.style.transitionDelay = `${Math.min(index % 3, 2) * 70}ms`;
    observer.observe(item);
  });
}

function visibilityController(element, onVisible, onHidden) {
  if (!("IntersectionObserver" in window)) {
    onVisible();
    return;
  }
  const observer = new IntersectionObserver(
    ([entry]) => entry.isIntersecting ? onVisible() : onHidden(),
    { threshold: 0.18 },
  );
  observer.observe(element);
}

const mascotTransitionTimers = new WeakMap();

function setMascotState(scene, state) {
  const previousState = scene.dataset.state;
  scene.dataset.state = state;
  if (!previousState || previousState === state || reducedMotion) return;

  scene.classList.remove("pet-transitioning");
  window.requestAnimationFrame(() => scene.classList.add("pet-transitioning"));
  window.clearTimeout(mascotTransitionTimers.get(scene));
  mascotTransitionTimers.set(
    scene,
    window.setTimeout(() => scene.classList.remove("pet-transitioning"), 540),
  );
}

const heroStage = document.querySelector("[data-hero-demo]");
const heroPanel = heroStage?.querySelector(".lume-panel");

if (heroStage && heroPanel) {
  const runningLabel = heroStage.querySelector("[data-running-label]");
  const heroSteps = [
    ["idle", "Running", 1600],
    ["waking", "Running", 2100],
    ["reading", "Running", 2300],
    ["opening", "Running", 1850],
    ["running", "Running", 2900],
    ["permission", "Permission", 2400],
    ["error", "Error", 1700],
    ["running", "Running", 1800],
    ["complete", "Complete", 2500],
    ["closing", "Complete", 1250],
  ];
  let heroIndex = 0;
  let heroTimer;
  let heroVisible = false;

  function runHeroStep() {
    if (!heroVisible || reducedMotion || document.hidden) return;
    const [state, agentLabel, duration] = heroSteps[heroIndex];
    setMascotState(heroStage, state);
    if (runningLabel) runningLabel.textContent = agentLabel;
    heroIndex = (heroIndex + 1) % heroSteps.length;
    clearTimeout(heroTimer);
    heroTimer = window.setTimeout(runHeroStep, duration);
  }

  visibilityController(
    heroStage,
    () => { heroVisible = true; heroStage.classList.add("scene-active"); runHeroStep(); },
    () => { heroVisible = false; heroStage.classList.remove("scene-active"); clearTimeout(heroTimer); },
  );
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && heroVisible) runHeroStep();
  });

  if (!reducedMotion && window.matchMedia("(pointer: fine)").matches) {
    let frame = 0;
    heroStage.addEventListener("pointermove", (event) => {
      if (["idle", "waking", "closing"].includes(heroStage.dataset.state)) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const bounds = heroStage.getBoundingClientRect();
        const x = (event.clientX - bounds.left) / bounds.width - 0.5;
        const y = (event.clientY - bounds.top) / bounds.height - 0.5;
        heroPanel.style.transform = `rotateY(${x * 4 - 1}deg) rotateX(${-y * 3 + 1}deg) translate3d(${x * 4}px, ${y * 3 - 3}px, 0)`;
      });
    });
    heroStage.addEventListener("pointerleave", () => {
      cancelAnimationFrame(frame);
      heroPanel.style.removeProperty("transform");
    });
  }
}

const workflow = document.querySelector("[data-workflow]");

if (workflow) {
  const nodeElements = Object.fromEntries(
    [...workflow.querySelectorAll("[data-workflow-node]")].map((node) => [node.dataset.workflowNode, node]),
  );
  const roleTokens = [...workflow.querySelectorAll("[data-role]")];
  const firstLink = workflow.querySelector(".first-link");
  const secondLink = workflow.querySelector(".second-link");
  const autoSteps = ["planner-running", "planner-complete", "handoff-one", "implementer-running", "implementer-complete", "handoff-two", "reviewer-running", "complete"];
  const durations = {
    "planner-running": 2300,
    "planner-complete": 900,
    "handoff-one": 1550,
    "implementer-running": 2600,
    "implementer-complete": 900,
    "handoff-two": 1550,
    "reviewer-running": 2300,
    complete: 2200,
  };
  let workflowIndex = 0;
  let workflowTimer;
  let workflowVisible = false;

  function setNode(node, state, label) {
    node?.classList.toggle("current", state === "current");
    node?.classList.toggle("completed", state === "completed");
    node?.classList.toggle("queued", state === "queued");
    const status = node?.querySelector("[data-node-status]");
    if (status) status.innerHTML = state === "current" ? `<i></i>${label}` : label;
  }

  function renderWorkflow(state) {
    workflow.dataset.state = state;
    const progress = ["handoff-one", "implementer-running", "implementer-complete"].includes(state)
      ? "50%"
      : ["handoff-two", "reviewer-running", "complete"].includes(state)
        ? "100%"
        : "0%";
    workflow.style.setProperty("--workflow-progress", progress);
    firstLink?.classList.toggle("sending", state === "handoff-one");
    firstLink?.classList.toggle("waiting", ["planner-running", "planner-complete"].includes(state));
    secondLink?.classList.toggle("sending", state === "handoff-two");
    secondLink?.classList.toggle("waiting", !["implementer-complete", "handoff-two", "reviewer-running", "complete"].includes(state));

    const plannerDone = !["planner-running"].includes(state);
    const implementerActive = state === "implementer-running";
    const implementerDone = ["implementer-complete", "handoff-two", "reviewer-running", "complete"].includes(state);
    const reviewerActive = state === "reviewer-running";
    const reviewerDone = state === "complete";
    const activeRole = ["planner-running", "planner-complete"].includes(state)
      ? "planner"
      : ["handoff-one", "implementer-running", "implementer-complete"].includes(state)
        ? "implementer"
        : "reviewer";
    roleTokens.forEach((token) => token.classList.toggle("current", token.dataset.role === activeRole));
    setNode(nodeElements.planner, plannerDone ? "completed" : "current", plannerDone ? "Complete" : "Running");
    setNode(nodeElements.implementer, implementerDone ? "completed" : implementerActive ? "current" : "queued", implementerDone ? "Complete" : implementerActive ? "Running" : "Queued");
    setNode(nodeElements.reviewer, reviewerDone ? "completed" : reviewerActive ? "current" : "queued", reviewerDone ? "Complete" : reviewerActive ? "Reviewing" : "Queued");
  }

  function runWorkflowStep() {
    if (!workflowVisible || reducedMotion || document.hidden) return;
    if (workflowIndex >= autoSteps.length) workflowIndex = 0;
    const state = autoSteps[workflowIndex];
    renderWorkflow(state);
    workflowIndex = (workflowIndex + 1) % autoSteps.length;
    clearTimeout(workflowTimer);
    workflowTimer = window.setTimeout(runWorkflowStep, durations[state]);
  }

  visibilityController(
    workflow,
    () => { workflowVisible = true; workflow.classList.add("scene-active"); runWorkflowStep(); },
    () => { workflowVisible = false; workflow.classList.remove("scene-active"); clearTimeout(workflowTimer); },
  );
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && workflowVisible) runWorkflowStep();
  });

  if (!reducedMotion && window.matchMedia("(pointer: fine)").matches) {
    workflow.addEventListener("pointermove", (event) => {
      const bounds = workflow.getBoundingClientRect();
      workflow.style.setProperty("--pointer-x", `${event.clientX - bounds.left}px`);
      workflow.style.setProperty("--pointer-y", `${event.clientY - bounds.top}px`);
      workflow.style.setProperty("--pointer-active", "1");
    });
    workflow.addEventListener("pointerleave", () => workflow.style.setProperty("--pointer-active", "0"));
  }
}

const dockingDemo = document.querySelector("[data-docking]");

if (dockingDemo) {
  const terminal = dockingDemo.querySelector("[data-drag-terminal]");
  const handle = dockingDemo.querySelector("[data-drag-handle]");
  const canvas = dockingDemo.querySelector(".docking-canvas");
  const sourceTerminal = dockingDemo.querySelector(".terminal-a");
  const highlight = dockingDemo.querySelector(".dock-highlight");
  const dockingSteps = [
    ["separated", 1800],
    ["approaching", 1700],
    ["docked", 2200],
    ["moving", 2100],
    ["resizing", 2300],
  ];
  let dockingIndex = 0;
  let dockingTimer;
  let dockingVisible = false;
  let dragging = false;
  let pointerId;
  let dragOffsetX = 0;
  let dragOffsetY = 0;
  let nearRightEdge = false;

  function clearManualPosition() {
    terminal?.style.removeProperty("left");
    terminal?.style.removeProperty("top");
    terminal?.style.removeProperty("width");
    terminal?.style.removeProperty("height");
    highlight?.style.removeProperty("left");
    highlight?.style.removeProperty("top");
    highlight?.style.removeProperty("width");
    highlight?.style.removeProperty("height");
    dockingDemo.classList.remove("manual-docked", "drag-near");
  }

  function runDockingStep() {
    if (!dockingVisible || reducedMotion || document.hidden || dragging) return;
    const [phase, duration] = dockingSteps[dockingIndex];
    dockingDemo.dataset.phase = phase;
    dockingIndex = (dockingIndex + 1) % dockingSteps.length;
    clearTimeout(dockingTimer);
    dockingTimer = window.setTimeout(runDockingStep, duration);
  }

  function replayDocking() {
    clearTimeout(dockingTimer);
    clearManualPosition();
    dockingIndex = 0;
    runDockingStep();
  }

  visibilityController(
    dockingDemo,
    () => { dockingVisible = true; dockingDemo.classList.add("scene-active"); runDockingStep(); },
    () => { dockingVisible = false; dockingDemo.classList.remove("scene-active"); clearTimeout(dockingTimer); },
  );
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && dockingVisible) runDockingStep();
  });

  if (handle && terminal && canvas && sourceTerminal && !reducedMotion) {
    handle.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      clearTimeout(dockingTimer);
      clearManualPosition();
      dragging = true;
      pointerId = event.pointerId;
      handle.setPointerCapture(pointerId);
      const canvasBounds = canvas.getBoundingClientRect();
      const terminalBounds = terminal.getBoundingClientRect();
      dragOffsetX = event.clientX - terminalBounds.left;
      dragOffsetY = event.clientY - terminalBounds.top;
      terminal.style.left = `${terminalBounds.left - canvasBounds.left}px`;
      terminal.style.top = `${terminalBounds.top - canvasBounds.top}px`;
      terminal.style.width = `${terminalBounds.width}px`;
      terminal.style.height = `${terminalBounds.height}px`;
      dockingDemo.dataset.phase = "separated";
      dockingDemo.classList.add("manual-dragging");
    });

    handle.addEventListener("pointermove", (event) => {
      if (!dragging || event.pointerId !== pointerId) return;
      const canvasBounds = canvas.getBoundingClientRect();
      const width = terminal.offsetWidth;
      const height = terminal.offsetHeight;
      const x = Math.max(0, Math.min(canvas.clientWidth - width, event.clientX - canvasBounds.left - dragOffsetX));
      const y = Math.max(0, Math.min(canvas.clientHeight - height, event.clientY - canvasBounds.top - dragOffsetY));
      terminal.style.left = `${x}px`;
      terminal.style.top = `${y}px`;

      const sourceLeft = sourceTerminal.offsetLeft;
      const sourceTop = sourceTerminal.offsetTop;
      const sourceRight = sourceLeft + sourceTerminal.offsetWidth;
      const sourceBottom = sourceTop + sourceTerminal.offsetHeight;
      const verticalOverlap = Math.max(0, Math.min(sourceBottom, y + height) - Math.max(sourceTop, y));
      nearRightEdge = Math.abs(x - sourceRight) < 72 && verticalOverlap > Math.min(height, sourceTerminal.offsetHeight) * 0.32;
      dockingDemo.classList.toggle("drag-near", nearRightEdge);
      if (nearRightEdge && highlight) {
        highlight.style.left = `${sourceRight}px`;
        highlight.style.top = `${sourceTop}px`;
        highlight.style.width = `${Math.min(width, canvas.clientWidth - sourceRight - 8)}px`;
        highlight.style.height = `${sourceTerminal.offsetHeight}px`;
      }
    });

    function finishDrag(event) {
      if (!dragging || event.pointerId !== pointerId) return;
      dragging = false;
      dockingDemo.classList.remove("manual-dragging", "drag-near");
      if (handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);
      if (nearRightEdge) {
        terminal.style.removeProperty("left");
        terminal.style.removeProperty("top");
        terminal.style.removeProperty("width");
        terminal.style.removeProperty("height");
        dockingDemo.dataset.phase = "docked";
        dockingDemo.classList.add("manual-docked");
        dockingTimer = window.setTimeout(() => {
          dockingDemo.classList.remove("manual-docked");
          dockingIndex = 3;
          runDockingStep();
        }, 1900);
      } else {
        dockingTimer = window.setTimeout(replayDocking, 1400);
      }
      nearRightEdge = false;
    }

    handle.addEventListener("pointerup", finishDrag);
    handle.addEventListener("pointercancel", finishDrag);
  }
}

const agentRows = [...document.querySelectorAll(".agent-row")];
if (!reducedMotion && agentRows.length) {
  let focusedAgent = 0;
  agentRows[focusedAgent].classList.add("focused");
  window.setInterval(() => {
    agentRows[focusedAgent].classList.remove("focused");
    focusedAgent = (focusedAgent + 1) % agentRows.length;
    agentRows[focusedAgent].classList.add("focused");
  }, 2800);
}

const year = document.querySelector("[data-year]");
if (year) year.textContent = String(new Date().getFullYear());

for (const ambientScene of document.querySelectorAll(".phone-wrap")) {
  visibilityController(
    ambientScene,
    () => ambientScene.classList.add("scene-active"),
    () => ambientScene.classList.remove("scene-active"),
  );
}
