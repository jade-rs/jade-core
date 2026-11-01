import { test, type TestContext } from "node:test";
import assert from "node:assert/strict";
import { createDeferredCrdt, createCrdtReply } from "./capture.ts";

function setup(t: TestContext) {
  const calls: string[] = [];
  const messages: any[] = [];
  const errors: any[] = [];
  const tasks: { run: () => void; onYield: () => void }[] = [];
  let fail = false;
  const view = {
    beginCrdtEdit: (replacement: boolean, incremental: boolean) => calls.push(`begin:${replacement}:${incremental}`),
    commitCrdtEdit: () => { if (fail) throw new Error("capture failed"); calls.push("commit"); },
    finishCrdtEdit: () => calls.push("stage"),
    abortPendingCrdtEdit: () => calls.push("abort-current"),
    abortCrdtEdit: () => calls.push("abort"),
    history_state: () => ({ canUndo: true, canRedo: false }),
  };
  t.mock.timers.enable({ apis: ["setTimeout"] });
  // The host serializes these functions into a worker: exercise that boundary too.
  const deferred = (Function(`return (${createDeferredCrdt.toString()})`)() as typeof createDeferredCrdt)(
    () => view, (run, onYield) => tasks.push({ run, onYield }), message => errors.push(message),
  );
  const reply = (command = "insert-text", documentEdit = true) =>
    (Function(`return (${createCrdtReply.toString()})`)() as typeof createCrdtReply)(
      view, { id: 7, command, documentEdit }, message => { messages.push(message); calls.push("send"); }, deferred,
    );
  const edit = (command = "insert-text", projected = false) => {
    const r = reply(command); r.begin(projected);
    r.post({ type: "response", id: 7, payload: { canUndo: false, changedParagraphs: [1] } });
  };
  return { calls, messages, errors, tasks, reply, edit, deferred, fail: () => { fail = true; } };
}

test("input is acknowledged before a coalesced capture; timers only enqueue", t => {
  const s = setup(t);
  s.edit(); t.mock.timers.tick(300); s.edit();
  assert.deepEqual(s.calls, ["begin:false:true", "stage", "send", "begin:false:true", "stage", "send"]);
  assert.deepEqual(s.messages.at(-1).payload, { canUndo: true, canRedo: false, changedParagraphs: [1] });
  t.mock.timers.tick(349); assert.equal(s.tasks.length, 0);
  t.mock.timers.tick(1); assert.equal(s.calls.includes("commit"), false);
  s.tasks.shift()!.run(); assert.equal(s.calls.filter(c => c === "commit").length, 1);
});
for (const command of ["crdt-begin", "crdt-export", "crdt-updates", "crdt-facts", "crdt-import", "undo", "redo", "begin-undo-group", "end-undo-group", "move-caret", "set-selection-range", "preview-composition-text"]) {
  test(`pending capture precedes ${command}`, t => {
    const s = setup(t); s.edit(); s.reply(command, command === "preview-composition-text").begin();
    assert.equal(s.calls.at(-1), "commit"); t.mock.timers.tick(500); assert.equal(s.tasks.length, 0);
  });
}
test("native rendering reads preserve pending history without capturing", t => {
  const s = setup(t); s.edit();
  for (const command of ["render", "history-state", "document-snapshot", "export-docx", "caret-ui-state"]) {
    const r = s.reply(command, false); r.begin(); r.post({ type: "response", id: 7, payload: { canUndo: false, canRedo: true } });
    assert.deepEqual(s.messages.at(-1).payload, { canUndo: true, canRedo: false });
  }
  assert.equal(s.calls.includes("commit"), false);
});
test("yielded and obsolete jobs never capture new input early", t => {
  const s = setup(t); s.edit(); t.mock.timers.tick(350);
  const stale = s.tasks.shift()!; stale.onYield(); s.edit(); stale.run();
  assert.equal(s.calls.includes("commit"), false);
  t.mock.timers.tick(350); s.tasks.shift()!.run(); assert.equal(s.calls.filter(c => c === "commit").length, 1);
});
test("failed input rolls back only the current command", t => {
  const s = setup(t); s.edit(); const r = s.reply(); r.begin(); r.post({ type: "error", id: 7 });
  assert.deepEqual(s.calls.slice(-3), ["begin:false:true", "abort-current", "send"]);
  t.mock.timers.tick(350); s.tasks.shift()!.run(); assert.equal(s.calls.at(-1), "commit");
});
test("background failure preserves acknowledged input and blocks save and edits", t => {
  const s = setup(t); s.edit(); s.fail(); t.mock.timers.tick(350);
  assert.doesNotThrow(() => s.tasks.shift()!.run());
  assert.deepEqual(s.errors, [{ type: "crdt-error", error: "Error: capture failed" }]);
  assert.throws(() => s.reply("crdt-export", false).begin(), /capture failed/);
  assert.throws(() => s.reply().begin(), /capture failed/);
  assert.doesNotThrow(() => s.reply("render", false).begin());
  assert.equal(s.calls.includes("abort"), false);
});
test("replacement captures synchronously and projected input remains deferred", t => {
  const s = setup(t); s.edit(); s.edit("apply-package-edit");
  assert.deepEqual(s.calls.slice(-4), ["commit", "begin:true:false", "commit", "send"]);
  s.edit("apply-package-edit", true);
  assert.deepEqual(s.calls.slice(-3), ["begin:false:true", "stage", "send"]);
});
test("synchronous capture failure is rolled back before the error response", t => {
  const s = setup(t); const r = s.reply("apply-bold"); r.begin(); s.fail();
  assert.throws(() => r.post({ type: "response", id: 7 }), /capture failed/);
  r.post({ type: "error", id: 7 }); assert.deepEqual(s.calls, ["begin:false:false", "abort", "send"]);
});
