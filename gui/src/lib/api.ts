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
  /** The CLI's warning about stale locks, or null when there are none */
  staleWarning: string | null;
  /** Japan time of this fetch, e.g. "10/19 18:50" */
  fetchedAt: string;
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

/** The result of locking or unlocking one file, worded like the CLI's output. */
export type OpResult = {
  path: string;
  /** The Unity .meta partner added automatically */
  autoAdded: boolean;
  /** e.g. "ロックしました", "できません" */
  label: string;
  detail: string | null;
  ok: boolean;
  /** Our own lock that git lfs refused to remove; a forced unlock would go through */
  canForce: boolean;
};

/** Someone else's lock that a forced unlock would remove. */
export type OthersLock = {
  id: string;
  path: string;
  owner: string;
  lockedAt: string;
};

/** Lock files (absolute paths or relative to root), with their Unity .meta. */
export function lockFiles(root: string, paths: string[]): Promise<OpResult[]> {
  return invoke<OpResult[]>("lock_files", { root, paths });
}

/** Other people's locks a forced unlock of `paths` would remove, to show before asking. */
export function previewForceUnlock(root: string, paths: string[]): Promise<OthersLock[]> {
  return invoke<OthersLock[]>("preview_force_unlock", { root, paths });
}

/** Unlock files. With `force`, only the other people's locks listed in `confirmed` (IDs) may go. */
export function unlockFiles(root: string, paths: string[], force: boolean, confirmed: string[]): Promise<OpResult[]> {
  return invoke<OpResult[]>("unlock_files", { root, paths, force, confirmed });
}

/** The first dropped path that is a folder. */
export function droppedFolder(paths: string[]): Promise<string | null> {
  return invoke<string | null>("dropped_folder", { paths });
}
