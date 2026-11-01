import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { createCrdtReply, createDeferredCrdt } from "./capture.ts";

/** Deterministic state-machine PBT, independent of timers/closures in capture.ts.
 * The counters and transitions correspond to formal/Capture.lean. Every seed is
 * its own test, so Node reports a reproducible seed and operation prefix.
 */
const oracleCommands: string[] = [];
const oracleStates: string[] = [];

for (let seed = 1; seed <= 128; seed++) {
  test(`capture model trace seed=${seed}`, t => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    let random = seed;
    const next = (n: number) => {
      random ^= random << 13; random ^= random >>> 17; random ^= random << 5;
      return (random >>> 0) % n;
    };
    let now = 0, generation = 0, acknowledged = 0, captured = 0;
    let dirty = false, failed = false, failCapture = false;
    let timer: { due: number; generation: number } | undefined;
    const expectedJobs: number[] = [];
    const jobs: { run: () => void; onYield: () => void }[] = [];
    let nativeText = "", replicaText = "", commits = 0, expectedCommits = 0;
    let errors = 0, expectedErrors = 0, batches = 0;
    const tokens = ["a", "条", "😀", "e\u0301", "\n", "ｶ"];
    const view = {
      beginCrdtEdit() {}, finishCrdtEdit() {}, abortPendingCrdtEdit() {}, abortCrdtEdit() {},
      history_state: () => ({ canUndo: false, canRedo: true }),
      commitCrdtEdit() {
        commits++;
        if (failCapture) throw new Error("capture failed");
        replicaText = nativeText; batches = acknowledged;
      },
    };
    // Exercise the exact serialization boundary used by crdtWorkerBridge.
    const deferred = (Function(`return (${createDeferredCrdt.toString()})`)() as typeof createDeferredCrdt)(
      () => view, (run, onYield) => jobs.push({ run, onYield }), () => errors++,
    );
    const reply = (command: string, documentEdit = false) =>
      createCrdtReply(view, { command, id: 7, documentEdit }, message => {
        if (dirty && message.type === "response" && message.payload && "canUndo" in message.payload) {
          assert.deepEqual(message.payload, { canUndo: true, canRedo: false });
        }
      }, deferred);
    const record = (command: string) => {
      oracleCommands.push(command);
      oracleStates.push(`${generation} ${acknowledged} ${captured} ${+dirty} ${+failed}`);
    };
    record("reset");
    const modelFlush = () => {
      generation++; timer = undefined;
      if (failed || !dirty) return;
      expectedCommits++;
      if (failCapture) { failed = true; expectedErrors++; }
      else { captured = acknowledged; dirty = false; }
    };
    const schedule = () => { generation++; timer = { due: now + 350, generation }; };
    const trace: string[] = [];
    // Keep testing long healthy histories before also exploring persistent failure.
    for (let i = 0; i < 256; i++) {
      const action = next(i < 192 ? 9 : 10);
      trace.push(`${action}@${now}`);
      try {
        if (action === 0 || action === 1) {
          const r = reply(action === 0 ? "insert-text" : "apply-package-edit", true);
          if (failed) {
            assert.throws(() => r.begin(action === 1), /capture failed/);
            record("edit");
          }
          else {
            r.begin(action === 1);
            nativeText = next(3) === 0 ? Array.from(nativeText).slice(0, -1).join("")
              : nativeText + tokens[next(tokens.length)];
            acknowledged++; dirty = true; schedule(); record("edit");
            r.post({ type: "response", id: 7, payload: { canUndo: false, canRedo: true } });
          }
        } else if (action === 2) {
          const delta = [0, 349, 350, 351, 700][next(5)]!;
          now += delta;
          if (timer && timer.due <= now) { expectedJobs.push(timer.generation); timer = undefined; }
          t.mock.timers.tick(delta);
        } else if ((action === 3 || action === 4) && jobs.length) {
          // Retain callbacks: running stale/already-run work must also be harmless.
          const index = next(jobs.length), expected = expectedJobs[index]!;
          if (dirty && expected === generation) {
            if (action === 3) modelFlush(); else schedule();
          }
          record(action === 3 ? `job ${expected} ${+!failCapture}` : `yield ${expected}`);
          if (action === 3) jobs[index]!.run(); else jobs[index]!.onYield();
        } else if (action === 5) {
          const command = ["crdt-export", "crdt-import", "undo", "redo", "move-caret", "crdt-facts", "set-selection-range", "preview-composition-text"][next(8)]!;
          const r = reply(command, command === "preview-composition-text");
          // IME's document-edit check rejects a failed session before flushing.
          if (failed && command === "preview-composition-text") record("read");
          else { modelFlush(); record(`barrier ${+!failCapture}`); }
          if (failed) assert.throws(() => r.begin(), /capture failed/);
          else { r.begin(); assert.equal(replicaText, nativeText); }
        } else if (action === 6) {
          const r = reply("caret-ui-state"); r.begin();
          r.post({ type: "response", id: 7, payload: { canUndo: false, canRedo: true } });
        } else if (action === 7) {
          const r = reply("delete-text", true);
          if (failed) assert.throws(() => r.begin(), /capture failed/);
          else { r.begin(); r.post({ type: "error", id: 7 }); }
        } else if (action === 8) {
          if (failed) assert.throws(() => deferred.check(), /capture failed/);
          else assert.doesNotThrow(() => deferred.check());
        } else if (action === 9) failCapture = !failCapture;
        assert.equal(deferred.pending(), dirty);
        assert.equal(commits, expectedCommits);
        assert.equal(errors, expectedErrors);
        assert.equal(batches, captured);
        assert.equal(jobs.length, expectedJobs.length);
        assert.ok(captured <= acknowledged);
        if (!dirty) assert.equal(replicaText, nativeText);
        if (failed) assert.ok(dirty);
      } catch (error) {
        throw new Error(`seed=${seed} prefix=${trace.join(",")}`, { cause: error });
      }
    }
    // Final save must either include all accepted input or report persistent failure.
    modelFlush(); record(`barrier ${+!failCapture}`);
    if (failed) assert.throws(() => deferred.flush(), /capture failed/);
    else { deferred.flush(); assert.equal(replicaText, nativeText); }
    assert.equal(commits, expectedCommits);
    assert.equal(errors, expectedErrors);
  });
}

// Mandatory in the upstream proof CI job; the plain Node suite stays usable by
// hosts that do not install Lean. This runs all seeds in one Lean process.
test("PBT reference transitions agree with the proved Lean step", { skip: process.env.JADE_LEAN_ORACLE !== "1" }, () => {
  const result = spawnSync("lean", ["--run", "Capture.lean"], {
    cwd: fileURLToPath(new URL("../formal", import.meta.url)),
    input: `${oracleCommands.join("\n")}\n`, encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
  });
  assert.equal(result.error, undefined);
  assert.equal(result.status, 0, result.stderr || result.stdout);
  const states = result.stdout.trim().split("\n");
  assert.equal(states.length, oracleStates.length);
  for (let i = 0; i < states.length; i++) {
    assert.equal(states[i], oracleStates[i], `oracle transition=${i} command=${oracleCommands[i]}`);
  }
});
