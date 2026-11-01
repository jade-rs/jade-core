interface CrdtView {
  beginCrdtEdit(replacement: boolean, incremental: boolean): void;
  commitCrdtEdit(): void;
  finishCrdtEdit(): void;
  abortPendingCrdtEdit(): void;
  abortCrdtEdit(): void;
  history_state(): Record<string, unknown>;
}
interface Reply {
  type: string;
  id?: number;
  payload?: Record<string, unknown>;
  [key: string]: unknown;
}
/** Serialized into the worker. Timers only enqueue; WASM access remains serialized. */
export function createDeferredCrdt(
  getView: () => CrdtView | null,
  enqueue: (run: () => void, onYield: () => void) => void,
  notify: (message: Reply) => void,
) {
  let dirty = false;
  let failure: unknown;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let generation = 0;
  const cancel = () => {
    clearTimeout(timer);
    generation++;
  };
  const flush = () => {
    cancel();
    if (failure) throw failure;
    if (!dirty) return;
    try {
      getView()?.commitCrdtEdit();
      dirty = false;
    } catch (error) {
      // Keep acknowledged native input intact. Never save an older replica.
      failure = error;
      notify({ type: "crdt-error", error: String(error) });
      throw error;
    }
  };
  const schedule = () => {
    cancel();
    const expected = generation;
    timer = setTimeout(() => {
      if (!dirty || expected !== generation) return;
      enqueue(
        () => {
          if (!dirty || expected !== generation) return;
          try {
            flush();
          } catch {
            /* Reported above; barriers reject too. */
          }
        },
        () => {
          if (dirty && expected === generation) schedule();
        },
      );
    }, 350);
  };
  return {
    flush,
    pending: () => dirty,
    check() {
      if (failure) throw failure;
    },
    stage() {
      getView()?.finishCrdtEdit();
      dirty = true;
      schedule();
    },
  };
}

/** Serialized into the worker. It contains no references to module scope. */
export function createCrdtReply(
  view: CrdtView | null,
  event: { id: number; command: string; documentEdit?: boolean },
  send: (message: Reply, transfer?: Transferable[]) => void,
  deferred: ReturnType<typeof createDeferredCrdt>,
) {
  let pending = false;
  let delayed = false;
  return {
    begin(projected = false) {
      if (!view) return;
      if (event.documentEdit) deferred.check();
      delayed =
        ["insert-text", "delete-text", "paste-text"].includes(event.command) ||
        (event.command === "apply-package-edit" && projected);
      // Capture before commands that consume/change the replica or its binding.
      // In particular, flush BEFORE the first provisional IME preview exists.
      if (
        (!delayed && event.documentEdit) ||
        [
          "crdt-begin",
          "crdt-export",
          "crdt-updates",
          "crdt-import",
          "crdt-facts",
          "undo",
          "redo",
          "begin-undo-group",
          "end-undo-group",
          "move-caret",
          "set-selection-range",
        ].includes(event.command)
      )
        deferred.flush();
      if (
        !event.documentEdit ||
        [
          "preview-composition-text",
          "preview-mail-merge",
          "undo",
          "redo",
          "crdt-import",
          "crdt-facts",
        ].includes(event.command)
      )
        return;
      view.beginCrdtEdit(event.command === "apply-package-edit" && !projected, delayed);
      pending = true;
    },
    post(message: Reply, transfer?: Transferable[]) {
      if (pending && view && message.id === event.id) {
        if (message.type === "response") {
          if (delayed) deferred.stage();
          else view.commitCrdtEdit();
          pending = false;
          if (message.payload && typeof message.payload === "object") {
            const history = deferred.pending()
              ? { canUndo: true, canRedo: false }
              : view.history_state();
            message = { ...message, payload: { ...message.payload, ...history } };
          }
        } else if (message.type === "error") {
          if (delayed) view.abortPendingCrdtEdit();
          else view.abortCrdtEdit();
          pending = false;
        }
      }
      if (
        deferred.pending() &&
        message.type === "response" &&
        message.payload &&
        ("canUndo" in message.payload || "canRedo" in message.payload)
      ) {
        message = { ...message, payload: { ...message.payload, canUndo: true, canRedo: false } };
      }
      send(message, transfer);
    },
  };
}
