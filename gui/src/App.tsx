import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderGit2, FolderOpen, Lock, LockKeyhole, RefreshCw, Users } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { type Confirm, ConfirmDialog } from "@/components/confirm-dialog";
import { LockSection } from "@/components/lock-section";
import { DropOverlay, ErrorBox, Loading, NoticeBox, WarningBox, Welcome } from "@/components/notices";
import { type Results, ResultsPanel } from "@/components/results";
import { Button } from "@/components/ui/button";
import {
  type LockReport,
  type LockRow,
  type OpResult,
  type OthersLock,
  droppedFolder,
  initialFolder,
  listLocks,
  lockFiles,
  previewForceUnlock,
  unlockFiles,
} from "@/lib/api";
import { cn } from "@/lib/utils";

const LAST_FOLDER_KEY = "repo-tweaks.lastFolder";

function loadLastFolder(): string | null {
  try {
    return localStorage.getItem(LAST_FOLDER_KEY);
  } catch {
    return null;
  }
}

function saveLastFolder(dir: string) {
  try {
    localStorage.setItem(LAST_FOLDER_KEY, dir);
  } catch {
    // Remembering the folder is only a convenience.
  }
}

export function App() {
  // False until the start-up folder is known: the welcome screen waits, so it does not flash.
  const [ready, setReady] = useState(false);
  const [dir, setDir] = useState<string | null>(null);
  const [report, setReport] = useState<LockReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  // A lock / unlock in progress. The folder cannot change meanwhile, so its result and the
  // refresh after it always belong to the folder on screen.
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<Results | null>(null);
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  // Ignore answers to older requests when the folder changes while one is in flight.
  // (The Rust side runs git lfs one call at a time and stops a hung git after 60 seconds.)
  const requestId = useRef(0);
  const started = useRef(false);
  const busyRef = useRef(false);

  const refresh = useCallback(async (target: string) => {
    const id = ++requestId.current;
    setLoading(true);
    try {
      const result = await listLocks(target);
      if (id !== requestId.current) return;
      setReport(result);
      setError(null);
      // Remember only folders that opened, so a mistyped one is not reopened next time.
      saveLastFolder(target);
    } catch (e) {
      if (id !== requestId.current) return;
      // Keep the last list on screen: an older list is more useful than none.
      setError(String(e));
    } finally {
      if (id === requestId.current) setLoading(false);
    }
  }, []);

  const openFolder = useCallback(
    (target: string) => {
      if (busyRef.current) {
        setNotice("ロック・解除の途中です。終わってからフォルダを開いてください");
        return;
      }
      setReport(null);
      setError(null);
      setResults(null);
      setNotice(null);
      setDir(target);
      void refresh(target);
    },
    [refresh],
  );

  // A folder given on the command line wins over the remembered one. Once only (React runs effects
  // twice in development), and not when the user has already opened a folder in the meantime.
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    void initialFolder()
      .catch(() => null)
      .then((arg) => {
        setReady(true);
        const start = arg ?? loadLastFolder();
        if (start && requestId.current === 0) openFolder(start);
      });
  }, [openFolder]);

  const chooseFolder = useCallback(async () => {
    const picked = await open({ directory: true, title: "リポジトリのフォルダを選ぶ" });
    if (typeof picked === "string") openFolder(picked);
  }, [openFolder]);

  // Dropping a folder on the window opens it.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          setDragging(true);
        } else if (payload.type === "leave") {
          setDragging(false);
        } else if (payload.type === "drop") {
          setDragging(false);
          void droppedFolder(payload.paths).then((folder) => {
            if (folder) openFolder(folder);
            else setNotice("フォルダをドロップしてください（ファイルのロックは「ファイルをロック」からできます）");
          });
        }
      })
      .then((stop) => {
        // The effect may already be cleaned up (React runs it twice in development)
        if (cancelled) stop();
        else unlisten = stop;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [openFolder]);

  /** Run a lock / unlock, show what happened to each file, then reload the list. */
  const runOp = useCallback(
    async (title: string, op: () => Promise<OpResult[]>) => {
      if (!dir) return;
      busyRef.current = true;
      setBusy(true);
      setResults(null);
      setNotice(null);
      try {
        setResults({ title, rows: await op() });
      } catch (e) {
        setResults({ title, error: String(e) });
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
      void refresh(dir);
    },
    [dir, refresh],
  );

  const lockFromDialog = useCallback(async () => {
    if (!report) return;
    const picked = await open({ multiple: true, defaultPath: report.root, title: "ロックするファイルを選ぶ" });
    if (!picked || picked.length === 0) return;
    void runOp("ロック", () => lockFiles(report.root, picked));
  }, [report, runOp]);

  const unlockOwn = useCallback(
    (lock: LockRow) => {
      if (!report) return;
      void runOp("解除", () => unlockFiles(report.root, [lock.path], false, []));
    },
    [report, runOp],
  );

  // Our own lock that git lfs refused (uncommitted changes, deleted file): remove it by ID.
  const forceOwn = useCallback(
    (path: string) => {
      if (!report) return;
      setConfirm({
        title: "変更を残したまま解除しますか？",
        message:
          "未コミットの変更や、手元に無いファイルがあっても、自分のロックを解除します。ファイルの変更はそのまま残ります。解除したあとは、ほかの人がロックして編集できるようになります。",
        items: [{ path }],
        confirmLabel: "解除する",
        onConfirm: () => {
          setConfirm(null);
          void runOp("解除", () => unlockFiles(report.root, [path], true, []));
        },
      });
    },
    [report, runOp],
  );

  // Someone else's lock: show exactly what would go (with its .meta), then remove only those.
  const forceOthers = useCallback(
    async (lock: LockRow) => {
      if (!report) return;
      busyRef.current = true;
      setBusy(true);
      setNotice(null);
      let others: OthersLock[];
      try {
        others = await previewForceUnlock(report.root, [lock.path]);
      } catch (e) {
        setResults({ title: "強制解除", error: String(e) });
        return;
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
      if (others.length === 0) {
        setNotice(`${lock.path} のロックは、すでに解除されています`);
        if (dir) void refresh(dir);
        return;
      }
      setConfirm({
        title: "他人のロックを強制解除しますか？",
        message:
          "ロックしている人の作業が失われるおそれがあります。先に本人に確かめてください。GitHub では、リポジトリの管理者（admin）だけが強制解除できます。",
        items: others.map((o) => ({ path: o.path, detail: `${o.owner}  ${o.lockedAt} から` })),
        confirmLabel: "強制解除する",
        onConfirm: () => {
          setConfirm(null);
          const ids = others.map((o) => o.id);
          void runOp("強制解除", () => unlockFiles(report.root, [lock.path], true, ids));
        },
      });
    },
    [report, dir, runOp, refresh],
  );

  const closeConfirm = useCallback(() => setConfirm(null), []);

  // Ctrl/Cmd+R and F5 reload the lock list instead of the page.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const reload = e.key === "F5" || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "r");
      if (!reload) return;
      e.preventDefault();
      if (dir && !loading && !busy) void refresh(dir);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dir, loading, busy, refresh]);

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-3 border-b border-zinc-200 bg-white/80 px-5 py-3 backdrop-blur dark:border-zinc-800 dark:bg-zinc-900/80">
        <div className="flex size-8 items-center justify-center rounded-lg bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900">
          <FolderGit2 className="size-4.5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold">{report?.name ?? "repo-tweaks"}</div>
          <div className="truncate text-xs text-zinc-500 select-text" title={report?.root ?? dir ?? undefined}>
            {report?.root ?? dir ?? "フォルダが選ばれていません"}
          </div>
        </div>
        {report && <span className="hidden text-xs text-zinc-500 md:inline">{report.fetchedAt} に更新</span>}
        <Button
          variant="ghost"
          size="icon"
          onClick={() => dir && void refresh(dir)}
          disabled={!dir || loading || busy}
          title="更新（Ctrl+R / ⌘R）"
          aria-label="更新"
        >
          <RefreshCw className={cn((loading || busy) && "animate-spin")} />
        </Button>
        <Button variant="outline" size="sm" onClick={() => void lockFromDialog()} disabled={!report || busy}>
          <LockKeyhole />
          ファイルをロック
        </Button>
        <Button variant="outline" size="sm" onClick={() => void chooseFolder()} disabled={busy}>
          <FolderOpen />
          フォルダを開く
        </Button>
      </header>

      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-5xl flex-col gap-5 p-5">
          {ready && !dir && <Welcome onOpen={() => void chooseFolder()} />}
          {notice && <NoticeBox message={notice} />}
          {error && (
            <ErrorBox
              message={error}
              note={report ? `${report.fetchedAt} に取得した一覧を表示しています` : undefined}
            />
          )}
          {results && (
            <ResultsPanel results={results} onClose={() => setResults(null)} onForce={forceOwn} disabled={busy} />
          )}
          {dir && !report && !error && loading && <Loading />}
          {report && (
            <div className={cn("flex flex-col gap-5 transition-opacity", loading && "opacity-60")}>
              {report.staleWarning && <WarningBox message={report.staleWarning} />}
              <LockSection
                icon={<Lock className="size-4" />}
                title="自分のロック"
                locks={report.ours}
                showOwner={false}
                action={{ label: "解除", onClick: unlockOwn }}
                disabled={busy || loading}
              />
              <LockSection
                icon={<Users className="size-4" />}
                title="他人のロック"
                locks={report.theirs}
                showOwner
                action={{ label: "強制解除", danger: true, onClick: (lock) => void forceOthers(lock) }}
                disabled={busy || loading}
              />
            </div>
          )}
        </div>
      </main>

      {dragging && <DropOverlay />}
      {confirm && <ConfirmDialog confirm={confirm} onCancel={closeConfirm} />}
    </div>
  );
}
