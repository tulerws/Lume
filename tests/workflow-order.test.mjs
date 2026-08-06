import assert from "node:assert/strict";
import {
  adjacentTerminalOnSide,
  orderTerminalsByPosition,
  orderWorkflowSteps,
} from "../src/lib/workflowOrder.ts";

function terminal(label, x, y, width = 320, height = 420) {
  return {
    label,
    sessionId: label,
    sessionNativeId: label,
    x,
    y,
    width,
    height,
    groupId: "group",
  };
}

function step(id) {
  return { id, sessionNativeId: id };
}

const left = terminal("left", 20, 42);
const right = terminal("right", 340, 36);
assert.deepEqual(
  orderTerminalsByPosition([right, left]).map((item) => item.label),
  ["left", "right"],
  "horizontal terminals should read left to right even with a small y offset",
);

const top = terminal("top", 30, 20, 300, 240);
const bottom = terminal("bottom", 34, 260, 300, 240);
assert.deepEqual(
  orderTerminalsByPosition([bottom, top]).map((item) => item.label),
  ["top", "bottom"],
  "vertical terminals should read top to bottom",
);

const horizontalSteps = [step("left"), step("right")];
assert.deepEqual(
  orderWorkflowSteps(horizontalSteps, [right, left], []).map((item) => item.id),
  ["left", "right"],
  "position should define the default workflow order",
);
assert.deepEqual(
  orderWorkflowSteps(horizontalSteps, [right, left], [{ fromStepId: "right", toStepId: "left" }]).map((item) => item.id),
  ["right", "left"],
  "an explicit connection direction should override physical order",
);

assert.equal(
  adjacentTerminalOnSide(left, "right", [left, right])?.label,
  "right",
  "the direction marker should resolve the terminal on the connected side",
);

console.log("workflow order test suite passed");
