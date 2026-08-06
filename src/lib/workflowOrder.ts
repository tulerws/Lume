import type {
  DockSide,
  TerminalWindowState,
  WorkflowConnectionDefinition,
  WorkflowStepDefinition,
} from "./domain";

export function terminalWorkflowKey(terminal: TerminalWindowState): string {
  return terminal.sessionNativeId?.trim() || terminal.sessionId;
}

function overlap(startA: number, endA: number, startB: number, endB: number): number {
  return Math.max(0, Math.min(endA, endB) - Math.max(startA, startB));
}

export function orderTerminalsByPosition(
  terminals: TerminalWindowState[],
): TerminalWindowState[] {
  if (terminals.length < 2) return [...terminals];
  const centerX = (terminal: TerminalWindowState) => terminal.x + terminal.width / 2;
  const centerY = (terminal: TerminalWindowState) => terminal.y + terminal.height / 2;
  const xValues = terminals.map(centerX);
  const yValues = terminals.map(centerY);
  const xSpread = Math.max(...xValues) - Math.min(...xValues);
  const ySpread = Math.max(...yValues) - Math.min(...yValues);
  const averageWidth = terminals.reduce((sum, terminal) => sum + terminal.width, 0) / terminals.length;
  const averageHeight = terminals.reduce((sum, terminal) => sum + terminal.height, 0) / terminals.length;
  const horizontal = ySpread <= averageHeight * 0.45;
  const vertical = xSpread <= averageWidth * 0.45;
  return [...terminals].sort((left, right) => {
    if (horizontal && !vertical) {
      return centerX(left) - centerX(right) || centerY(left) - centerY(right);
    }
    if (vertical && !horizontal) {
      return centerY(left) - centerY(right) || centerX(left) - centerX(right);
    }
    return centerY(left) - centerY(right) || centerX(left) - centerX(right);
  });
}

export function orderWorkflowSteps(
  steps: WorkflowStepDefinition[],
  terminals: TerminalWindowState[],
  connections: WorkflowConnectionDefinition[],
): WorkflowStepDefinition[] {
  const originalIndex = new Map(steps.map((step, index) => [step.id, index]));
  const spatialIndex = new Map(
    orderTerminalsByPosition(terminals).map((terminal, index) => [terminalWorkflowKey(terminal), index]),
  );
  const compareSteps = (left: WorkflowStepDefinition, right: WorkflowStepDefinition) => {
    const leftPosition = spatialIndex.get(left.sessionNativeId);
    const rightPosition = spatialIndex.get(right.sessionNativeId);
    if (leftPosition !== undefined && rightPosition !== undefined) {
      return leftPosition - rightPosition;
    } else if (leftPosition !== undefined) {
      return -1;
    } else if (rightPosition !== undefined) {
      return 1;
    }
    return (originalIndex.get(left.id) ?? 0) - (originalIndex.get(right.id) ?? 0);
  };

  const stepById = new Map(steps.map((step) => [step.id, step]));
  const indegree = new Map(steps.map((step) => [step.id, 0]));
  const outgoing = new Map<string, string[]>();
  for (const connection of connections) {
    if (!stepById.has(connection.fromStepId) || !stepById.has(connection.toStepId)) continue;
    outgoing.set(connection.fromStepId, [
      ...(outgoing.get(connection.fromStepId) ?? []),
      connection.toStepId,
    ]);
    indegree.set(connection.toStepId, (indegree.get(connection.toStepId) ?? 0) + 1);
  }

  const ready = steps.filter((step) => indegree.get(step.id) === 0).sort(compareSteps);
  const ordered: WorkflowStepDefinition[] = [];
  while (ready.length) {
    const step = ready.shift()!;
    ordered.push(step);
    for (const nextId of outgoing.get(step.id) ?? []) {
      const nextDegree = (indegree.get(nextId) ?? 0) - 1;
      indegree.set(nextId, nextDegree);
      if (nextDegree === 0) {
        const next = stepById.get(nextId);
        if (next) {
          ready.push(next);
          ready.sort(compareSteps);
        }
      }
    }
  }

  if (ordered.length < steps.length) {
    const included = new Set(ordered.map((step) => step.id));
    ordered.push(...steps.filter((step) => !included.has(step.id)).sort(compareSteps));
  }
  return ordered;
}

export function adjacentTerminalOnSide(
  current: TerminalWindowState,
  side: DockSide,
  terminals: TerminalWindowState[],
): TerminalWindowState | undefined {
  const candidates = terminals.filter((terminal) => {
    if (terminal.label === current.label || terminal.groupId !== current.groupId) return false;
    const verticalOverlap = overlap(
      current.y,
      current.y + current.height,
      terminal.y,
      terminal.y + terminal.height,
    );
    const horizontalOverlap = overlap(
      current.x,
      current.x + current.width,
      terminal.x,
      terminal.x + terminal.width,
    );
    if (side === "left" || side === "right") {
      if (verticalOverlap < Math.min(current.height, terminal.height) * 0.25) return false;
      return side === "left" ? terminal.x < current.x : terminal.x > current.x;
    }
    if (horizontalOverlap < Math.min(current.width, terminal.width) * 0.25) return false;
    return side === "top" ? terminal.y < current.y : terminal.y > current.y;
  });
  return candidates.sort((left, right) => {
    const leftGap = side === "left"
      ? current.x - (left.x + left.width)
      : side === "right"
        ? left.x - (current.x + current.width)
        : side === "top"
          ? current.y - (left.y + left.height)
          : left.y - (current.y + current.height);
    const rightGap = side === "left"
      ? current.x - (right.x + right.width)
      : side === "right"
        ? right.x - (current.x + current.width)
        : side === "top"
          ? current.y - (right.y + right.height)
          : right.y - (current.y + current.height);
    return Math.abs(leftGap) - Math.abs(rightGap);
  })[0];
}
