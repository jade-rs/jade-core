import assert from "node:assert/strict";
import { test } from "node:test";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { CrdtSession, binaryToBase64, base64ToBinary } from "./session.ts";

const bytes = (n: number) => new Uint8Array([n]);
const envelope = (n: number, seq = n) => ({ seq, version: binaryToBase64(bytes(n)), snapshot: binaryToBase64(bytes(n)) });
function setup() {
  const calls: (number | undefined)[] = [];
  const editor = {
    documentEditVersion: 1,
    exportCrdt: async () => ({ snapshot: bytes(1), version: bytes(1) }),
    importCrdtUpdates: async (_: Uint8Array) => { editor.documentEditVersion++; },
  };
  const transport = {
    send: async (_: Uint8Array) => envelope(2),
    load: async (seq?: number) => { calls.push(seq); return envelope(2); },
  };
  return { editor, transport, calls, session: new CrdtSession(editor, transport, undefined, 0) };
}
test("save imports a concurrent frontier once and advances only the clean marker", async () => {
  const { session, editor } = setup();
  assert.deepEqual(await session.save(), { version: 2, seq: 2 });
  assert.equal(await session.pull(2), null);
  assert.equal(editor.documentEditVersion, 2);
});
for (const phase of ["send", "import"] as const) test(`typing during ${phase} remains dirty`, async () => {
  const { session, editor, transport } = setup();
  if (phase === "send") transport.send = async () => { editor.documentEditVersion++; return envelope(2); };
  else editor.importCrdtUpdates = async () => { editor.documentEditVersion += 2; };
  assert.equal((await session.save()).version, 1);
  assert.equal(editor.documentEditVersion, 3);
});
test("a rejected save releases the queue for a pending pull and retry", async () => {
  const { session, transport, calls } = setup();
  transport.send = async () => { throw Error("offline"); };
  const saving = session.save();
  const pulling = session.pull(1);
  await assert.rejects(saving, /offline/);
  assert.equal(await pulling, 2);
  assert.deepEqual(calls, [0]);
  transport.send = async () => envelope(2, 3);
  assert.deepEqual(await session.save(), { version: 2, seq: 3 });
  assert.equal(await session.pull(2), null);
  assert.deepEqual(calls, [0, 3]);
});
test("failed imports retain the cursor and frontier so the next pull retries", async () => {
  const { session, editor, calls } = setup();
  editor.importCrdtUpdates = async () => { throw Error("invalid projection"); };
  await assert.rejects(session.pull(1), /invalid projection/);
  editor.importCrdtUpdates = async () => { editor.documentEditVersion++; };
  assert.equal(await session.pull(1), 2);
  assert.deepEqual(calls, [0, 0]);
  assert.equal(await session.pull(2), null);
  assert.deepEqual(calls, [0, 0, 2]);
});
test("slow pulls coalesce and release the queue for save, even after failure", async () => {
  const { session, transport } = setup();
  let reject!: (e: Error) => void;
  let loads = 0;
  transport.load = () => { loads++; return new Promise((_, r) => { reject = r; }); };
  const first = session.pull(1);
  assert.equal(session.pull(1), first);
  const save = session.save();
  await Promise.resolve();
  reject(Error("offline"));
  await assert.rejects(first, /offline/);
  assert.deepEqual(await save, { version: 2, seq: 2 });
  assert.equal(loads, 1);
});
test("identical frontier advances opaque transport cursor without importing", async () => {
  const { editor } = setup();
  const seen: (string | undefined)[] = [];
  const session = new CrdtSession(editor, {
    send: async () => ({ ...envelope(1), seq: "next" }),
    load: async seq => { seen.push(seq); return null; },
  }, envelope(1).version, "start");
  await session.save(); await session.pull(1);
  assert.deepEqual(seen, ["next"]);
  assert.equal(editor.documentEditVersion, 1);
});
test("large snapshots and all byte values round-trip", () => {
  const source = Uint8Array.from({ length: 262401 }, (_, i) => i % 256);
  assert.deepEqual(base64ToBinary(binaryToBase64(source)), source);
  assert.deepEqual(base64ToBinary(""), bytes(0).subarray(0, 0));
  assert.throws(() => base64ToBinary("!"));
});

test("PBT: retry cursors, duplicate frontiers and clean markers (128 seeds x 128 pulls)", async () => {
  const oracle: string[] = [];
  const expected: string[] = [];
  for (let seed = 1; seed <= 128; seed++) {
    let rng = seed;
    const next = () => { rng = (Math.imul(rng, 1664525) + 1013904223) >>> 0; return rng; };
    const { editor } = setup();
    let cursor = 0, seen = "", fail = false, during = 0, remote = 1, seq = 1;
    let expectedCursor = 0;
    const session = new CrdtSession(editor, {
      send: async () => envelope(remote, seq),
      load: async value => { assert.equal(value, expectedCursor, `seed=${seed}`); return envelope(remote, seq); },
    }, undefined, cursor);
    editor.importCrdtUpdates = async () => {
      if (fail) throw Error("retry");
      editor.documentEditVersion += 1 + during;
    };
    for (let step = 0; step < 128; step++) {
      editor.documentEditVersion += next() % 3;
      const before = editor.documentEditVersion;
      const clean = Math.max(0, before - (next() % 3));
      fail = next() % 5 === 0; during = next() % 3;
      remote = 1 + next() % 100; seq++;
      const version = envelope(remote).version;
      const duplicate = version === seen;
      if (fail && !duplicate) {
        await assert.rejects(session.pull(clean), /retry/);
        assert.equal(editor.documentEditVersion, before);
        oracle.push(`${before} ${before} ${clean} ${cursor} ${seq} 0`);
        expected.push(`0 ${cursor}`);
        continue;
      }
      const result = await session.pull(clean);
      const oldCursor = cursor;
      expectedCursor = seq; cursor = seq;
      if (duplicate) assert.equal(result, null);
      else {
        const after = before + 1 + during;
        const marker = before === clean && during === 0 ? after : null;
        assert.equal(result, marker, `seed=${seed} step=${step}`);
        oracle.push(`${before} ${after} ${clean} ${oldCursor} ${seq} 1`);
        expected.push(`${marker ?? 0} ${seq}`);
      }
      seen = version;
    }
  }
  if (process.env.JADE_LEAN_ORACLE === "1") {
    const proof = spawnSync("lean", ["--run", "Session.lean"], {
      cwd: fileURLToPath(new URL("../formal/", import.meta.url)), input: oracle.join("\n") + "\n", encoding: "utf8", maxBuffer: 4 * 1024 * 1024,
    });
    assert.equal(proof.status, 0, proof.stderr || proof.stdout);
    assert.deepEqual(proof.stdout.trim().split("\n"), expected);
  }
});
