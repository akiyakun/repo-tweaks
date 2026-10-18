import { invoke } from "@tauri-apps/api/core";

/** One lock, already formatted for display by the Rust side (same wording as the CLI). */
export type LockRow = {
  id: string;
  path: string;
  owner: string;
  /** Japan time, e.g. "10/18 01:30" */
  lockedAt: string;
  /** e.g. "1日2時間" */
  age: string;
  stale: boolean;
};

export type LockReport = {
  /** Repository root as git prints it */
  root: string;
  name: string;
  staleDays: number;
  staleCount: number;
  ours: LockRow[];
  theirs: LockRow[];
};

export function listLocks(dir: string): Promise<LockReport> {
  return invoke<LockReport>("list_locks", { dir });
}

/** The folder passed on the command line, if any. */
export function initialFolder(): Promise<string | null> {
  return invoke<string | null>("initial_folder");
}
