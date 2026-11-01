/** Host-independent persistence session. It does not start polling or networking. */
export interface CrdtEnvelope<Cursor> {
  seq: Cursor;
  version: string;
  snapshot: string;
}
export interface CrdtExport {
  snapshot: Uint8Array;
  version: Uint8Array;
}

export function binaryToBase64(bytes: Uint8Array): string {
  let text = "";
  for (let i = 0; i < bytes.length; i += 8192)
    text += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return btoa(text);
}
export function base64ToBinary(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
interface Replica {
  readonly documentEditVersion: number;
  exportCrdt(): Promise<CrdtExport>;
  importCrdtUpdates(updates: Uint8Array): Promise<unknown>;
}
/** Serialize network/import operations, while allowing local typing to continue.
 * SDK versions include remote imports, so only a lone import can advance a clean marker. */
export class CrdtSession<Cursor = number, Note = string> {
  private readonly editor: Replica;
  private readonly transport: {
    send(snapshot: Uint8Array, note?: Note): Promise<CrdtEnvelope<Cursor>>;
    load(seq?: Cursor): Promise<CrdtEnvelope<Cursor> | null>;
  };
  private tail: Promise<unknown> = Promise.resolve();
  private seen: string | undefined;
  private seq: Cursor | undefined;
  private pulling: Promise<number | null> | null = null;
  constructor(
    editor: Replica,
    transport: {
      send(snapshot: Uint8Array, note?: Note): Promise<CrdtEnvelope<Cursor>>;
      load(seq?: Cursor): Promise<CrdtEnvelope<Cursor> | null>;
    },
    initialVersion?: string,
    initialSeq?: Cursor,
  ) {
    this.editor = editor;
    this.transport = transport;
    this.seen = initialVersion;
    this.seq = initialSeq;
  }
  private queue<T>(run: () => Promise<T>): Promise<T> {
    const next = this.tail.then(run);
    this.tail = next.catch(() => undefined);
    return next;
  }
  private async receive(bundle: CrdtEnvelope<Cursor>, cleanVersion: number | null) {
    if (this.seen === bundle.version) {
      this.seq = bundle.seq;
      return null;
    }
    const before = this.editor.documentEditVersion;
    await this.editor.importCrdtUpdates(base64ToBinary(bundle.snapshot));
    this.seen = bundle.version;
    this.seq = bundle.seq;
    const after = this.editor.documentEditVersion;
    return before === cleanVersion && after === before + 1 ? after : null;
  }
  save(note?: Note) {
    return this.queue(async () => {
      const version = this.editor.documentEditVersion;
      const local = await this.editor.exportCrdt();
      const bundle = await this.transport.send(local.snapshot, note);
      if (bundle.version === binaryToBase64(local.version)) {
        this.seq = bundle.seq;
        this.seen = bundle.version;
        return { version, seq: bundle.seq };
      }
      return { version: (await this.receive(bundle, version)) ?? version, seq: bundle.seq };
    });
  }
  pull(cleanVersion: number | null) {
    // A slow/offline transport must not enqueue one request per timer tick ahead
    // of the next save. All callers observe the same pending receive.
    if (!this.pulling) {
      this.pulling = this.queue(async () => {
        const bundle = await this.transport.load(this.seq);
        return bundle ? this.receive(bundle, cleanVersion) : null;
      }).finally(() => {
        this.pulling = null;
      });
    }
    return this.pulling;
  }
}
