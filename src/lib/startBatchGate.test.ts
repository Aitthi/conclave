// The repo carries no @types/node (and adding it needs a challenge, plan R4):
// Node's own runner supplies these modules at run time, so tsc sees them untyped.
// @ts-expect-error -- no @types/node; resolved by Node at run time
import { test } from "node:test";
// @ts-expect-error -- no @types/node; resolved by Node at run time
import assert from "node:assert/strict";
import { blockedByStartBatch } from "./startBatchGate.ts";

const batch = {
  readyAgentIds: ["ready"],
  skippedStoppedAgentIds: ["stopped"],
  failures: [{ workspaceAgentId: "failed", error: "spawn failed" }],
};

test("no batch never blocks", () => {
  assert.equal(blockedByStartBatch(null, "any"), false);
});

test("ready id is not blocked", () => {
  assert.equal(blockedByStartBatch(batch, "ready"), false);
});

test("failed id is blocked", () => {
  assert.equal(blockedByStartBatch(batch, "failed"), true);
});

test("skipped-stopped id is blocked", () => {
  assert.equal(blockedByStartBatch(batch, "stopped"), true);
});

test("id added after Start (unknown to the batch) is not blocked", () => {
  assert.equal(blockedByStartBatch(batch, "added-after-start"), false);
});
