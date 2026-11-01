import assert from "node:assert/strict";
import { stripTypeScriptTypes } from "node:module";
import { test } from "node:test";
import { crdtSdkBridge, crdtWorkerBridge } from "./capture.ts";

const workerBoundaries = [
  "        run: async () => {\n            await wasmReady;",
  "                const readRunProperties = () => {",
  "            case 'apply-package-edit': {",
];
const sdkBoundaries = [
  "    const documentEdit = DOCUMENT_EDIT_COMMANDS.has(command);",
  "      if (!id) {\n        return;\n      }\n      const entry = this.pending.get(id);",
  "  async applyPackageEdit(bytes: Uint8Array, expected: Uint8Array) {",
];

test("SDK and worker fail closed for missing, moved or ambiguous boundaries", () => {
  for (const [adapt, boundaries] of [[crdtWorkerBridge, workerBoundaries], [crdtSdkBridge, sdkBoundaries]] as const) {
    for (const boundary of boundaries) {
      const rest = boundaries.filter(b => b !== boundary).join("\n");
      for (const source of [rest, `${rest}\n${boundary}\n${boundary}`, `${rest}\n${boundary.replace(/ /, "\t")}`]) {
        assert.throws(() => adapt(source), /bridge needs updating/);
      }
    }
    const patched = adapt(boundaries.join("\n"));
    assert.throws(() => adapt(patched), /bridge needs updating/);
  }
});

test("SDK flushes before export, routes all CRDT commands and applies imported pages only on success", async () => {
  const source = crdtSdkBridge(`
const DOCUMENT_EDIT_COMMANDS = new Set();
function classify(command) {
    const documentEdit = DOCUMENT_EDIT_COMMANDS.has(command);
    return documentEdit;
}
class Editor {
  receive(event) {
    const {type, id} = event.data;
      if (!id) {
        return;
      }
      const entry = this.pending.get(id);
    return entry;
  }
  async applyPackageEdit(bytes: Uint8Array, expected: Uint8Array) {}
}
`);
  const { Editor, classify } = Function(`${stripTypeScriptTypes(source)}; return { Editor, classify };`)();
  const editor = new Editor(), calls: unknown[] = [];
  let reject = false;
  const payload = { pages: ["page"] };
  editor.ready = async () => { calls.push("ready"); };
  editor.flushPendingEdits = async () => { calls.push("flush"); };
  editor.notifyDocumentChange = (command: string) => calls.push(command);
  editor.workerState = {
    controller: { invoke: async (command: string, args: unknown) => {
      calls.push([command, args]); if (reject) throw new Error("projection failed"); return payload;
    } },
    interaction: { applyUpdatePayload: (p: unknown) => calls.push(p) },
  };
  const snapshot = new Uint8Array([1]), updates = new Uint8Array([2]);
  assert.equal(classify("crdt-import"), true);
  assert.equal(classify("crdt-facts"), true);
  assert.equal(classify("crdt-export"), false);
  await editor.beginCrdt("doc", "base", "actor", snapshot);
  await editor.exportCrdt();
  await editor.crdtUpdatesSince(snapshot);
  await editor.appendCrdtFacts([{ opaque: "host metadata" }]);
  assert.equal(await editor.importCrdtUpdates(updates), payload);
  assert.deepEqual(calls, [
    "ready", ["crdt-begin", { document: "doc", baseline: "base", actor: "actor", snapshot }],
    "ready", "flush", ["crdt-export", undefined],
    "ready", ["crdt-updates", { version: snapshot }],
    "ready", ["crdt-facts", { values: [{ opaque: "host metadata" }] }],
    "ready", ["crdt-import", { updates }], payload,
  ]);
  calls.length = 0; reject = true;
  await assert.rejects(editor.importCrdtUpdates(updates), /projection failed/);
  assert.deepEqual(calls, ["ready", ["crdt-import", { updates }]]);
  editor.receive({ data: { type: "crdt-error" } });
  assert.equal(calls.at(-1), "crdt-error");
});

test("generated worker uses the shared scheduler and preserves bitmap transfers and capture barriers", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const tasks: { command?: string; run: () => Promise<void> | void; onYield?: () => void }[] = [];
  const messages: unknown[][] = [], calls: unknown[] = [];
  const bitmap = {}, payload = { pages: [{ bitmap }], canUndo: false };
  const view = {
    beginCrdt: (...args: unknown[]) => calls.push(["begin", ...args]),
    exportCrdt: () => { calls.push("export"); return "snapshot"; },
    crdtUpdatesSince: (version: unknown) => version,
    appendCrdtFacts: (values: unknown) => values,
    importCrdtUpdates: async () => payload,
    clear_cached_pages: () => calls.push("clear"),
    beginCrdtEdit: (...args: unknown[]) => calls.push(["edit", ...args]),
    finishCrdtEdit: () => calls.push("stage"),
    commitCrdtEdit: () => calls.push("commit"),
    history_state: () => ({ canUndo: true, canRedo: false }),
  };
  const worker = crdtWorkerBridge(`
const wasmReady = Promise.resolve();
self.onmessage = event => {
  const {command, id} = event.data;
  scheduler.enqueue({
        run: async () => {
            await wasmReady;
                const readRunProperties = () => {};
          switch (command) {
            case 'apply-package-edit': {
              break;
            }
            case 'insert-text': {
              postMessage({type: 'response', id, payload: {canUndo: false}}); break;
            }
          }
        }
  });
};`);
  const self: { postMessage: (...args: unknown[]) => void; onmessage?: (event: unknown) => void } = {
    postMessage: (...args) => messages.push(args),
  };
  Function("self", "documentView", "scheduler", "attachPageCount", "collectBitmapTransfers", worker)(
    self, view, { _seq: 0, enqueue: (task: typeof tasks[number]) => tasks.push(task) },
    (p: unknown) => p, () => [bitmap],
  );
  async function dispatch(command: string, args: Record<string, unknown> = {}) {
    self.onmessage!({ data: { command, id: 7, ...args } });
    await tasks.pop()!.run();
  }
  await dispatch("crdt-begin", { document: "doc", baseline: "base", actor: "actor" });
  assert.deepEqual(calls, [["begin", "doc", "base", "actor", undefined]]);
  await dispatch("insert-text", { documentEdit: true });
  assert.deepEqual(calls.slice(-2), [["edit", false, true], "stage"]);
  assert.deepEqual(messages.at(-1), [{ type: "response", id: 7, payload: { canUndo: true, canRedo: false } }, undefined]);
  t.mock.timers.tick(350);
  assert.equal(tasks[0]!.command, "internal-crdt-capture");
  await dispatch("crdt-export");
  assert.deepEqual(calls.slice(-2), ["commit", "export"]);
  await tasks.shift()!.run(); // Already-invalidated capture must do nothing.
  assert.equal(calls.filter(c => c === "commit").length, 1);
  await dispatch("crdt-import", { updates: new Uint8Array([1]) });
  assert.deepEqual(messages.at(-1), [{ type: "response", command: "crdt-import", id: 7, payload }, [bitmap]]);
  assert.equal(calls.at(-1), "clear");
});
