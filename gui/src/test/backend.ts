// A fake Rust side for screen tests: answers the app's commands from `handlers` and records calls.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { LockReport, LockRow, OpReport, OpResult } from "@/lib/api";

export type Call = { cmd: string; args: Record<string, unknown> };
type Handler = (args: Record<string, unknown>) => unknown;

/** Install the fake. A command without a handler fails the test loudly. */
export function fakeBackend(handlers: Record<string, Handler>): Call[] {
  const calls: Call[] = [];
  mockWindows("main");
  mockIPC(
    (cmd, args) => {
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> });
      const handler = handlers[cmd];
      if (!handler) throw new Error(`unexpected command: ${cmd}`);
      return handler((args ?? {}) as Record<string, unknown>);
    },
    { shouldMockEvents: true },
  );
  return calls;
}

/** Commands answered with an error message, as Tauri rejects with the Err(String) of Rust. */
export function fail(message: string): Promise<never> {
  return Promise.reject(message);
}

export function callsTo(calls: Call[], cmd: string): Record<string, unknown>[] {
  return calls.filter((c) => c.cmd === cmd).map((c) => c.args);
}

/** Drop paths on the window, as the webview's drag and drop events do. */
export async function dropPaths(paths: string[]) {
  await emit("tauri://drag-enter", { paths, position: { x: 10, y: 10 } });
  await emit("tauri://drag-drop", { paths, position: { x: 10, y: 10 } });
}

export function lock(path: string, owner: string, extra: Partial<LockRow> = {}): LockRow {
  return { id: `id-${path}`, path, owner, lockedAt: "10/20 13:01", age: "2時間", stale: false, ...extra };
}

export function report(extra: Partial<LockReport> = {}): LockReport {
  return {
    root: "/work/game",
    name: "game",
    staleDays: 3,
    staleCount: 0,
    staleWarning: null,
    fetchedAt: "10/20 15:00",
    ours: [],
    theirs: [],
    ...extra,
  };
}

export function result(path: string, label: string, extra: Partial<OpResult> = {}): OpResult {
  return { path, autoAdded: false, label, detail: null, ok: true, canForce: false, ...extra };
}

export function opReport(results: OpResult[], notice: OpReport["notice"] = null): OpReport {
  return { results, notice };
}
