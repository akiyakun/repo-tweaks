import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, FolderGit2, FolderOpen, Lock, RefreshCw, Users } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { type LockReport, type LockRow, initialFolder, listLocks } from "@/lib/api";
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
  const [dir, setDir] = useState<string | null>(loadLastFolder);
  const [report, setReport] = useState<LockReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [fetchedAt, setFetchedAt] = useState<Date | null>(null);
  // Ignore answers to older requests when the folder changes while one is in flight.
  const requestId = useRef(0);

  const refresh = useCallback(async (target: string) => {
    const id = ++requestId.current;
    setLoading(true);
    try {
      const result = await listLocks(target);
      if (id !== requestId.current) return;
      setReport(result);
      setError(null);
      setFetchedAt(new Date());
    } catch (e) {
      if (id !== requestId.current) return;
      setReport(null);
      setError(String(e));
      setFetchedAt(null);
    } finally {
      if (id === requestId.current) setLoading(false);
    }
  }, []);

  // A folder given on the command line wins over the remembered one.
  useEffect(() => {
    void initialFolder().then((arg) => {
      if (arg) {
        saveLastFolder(arg);
        setDir(arg);
      }
    });
  }, []);

  useEffect(() => {
    if (dir) void refresh(dir);
  }, [dir, refresh]);

  const chooseFolder = useCallback(async () => {
    const picked = await open({ directory: true, title: "リポジトリのフォルダを選ぶ" });
    if (typeof picked !== "string") return;
    saveLastFolder(picked);
    setReport(null);
    setError(null);
    setDir(picked);
    // Same folder again: the effect does not fire, so reload explicitly.
    if (picked === dir) void refresh(picked);
  }, [dir, refresh]);

  // Ctrl/Cmd+R and F5 reload the lock list instead of the page.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const reload = e.key === "F5" || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "r");
      if (!reload) return;
      e.preventDefault();
      if (dir && !loading) void refresh(dir);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dir, loading, refresh]);

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
        {fetchedAt && (
          <span className="hidden text-xs text-zinc-500 sm:inline">
            {fetchedAt.toLocaleTimeString("ja-JP", { hour: "2-digit", minute: "2-digit" })} に更新
          </span>
        )}
        <Button
          variant="ghost"
          size="icon"
          onClick={() => dir && void refresh(dir)}
          disabled={!dir || loading}
          title="更新（Ctrl+R / ⌘R）"
          aria-label="更新"
        >
          <RefreshCw className={cn(loading && "animate-spin")} />
        </Button>
        <Button variant="outline" size="sm" onClick={() => void chooseFolder()}>
          <FolderOpen />
          フォルダを開く
        </Button>
      </header>

      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-5xl flex-col gap-5 p-5">
          {!dir && <Welcome onOpen={() => void chooseFolder()} />}
          {error && <ErrorBox message={error} />}
          {dir && !report && !error && loading && <Loading />}
          {report && (
            <div className={cn("flex flex-col gap-5 transition-opacity", loading && "opacity-60")}>
              {report.staleCount > 0 && (
                <div className="flex items-center gap-2 rounded-lg border border-amber-200 bg-amber-50 px-4 py-2.5 text-sm text-amber-900 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200">
                  <AlertTriangle className="size-4 shrink-0" />
                  {report.staleDays}日以上ロックされたままのものが {report.staleCount} 件あります
                </div>
              )}
              <LockSection
                icon={<Lock className="size-4" />}
                title="自分のロック"
                locks={report.ours}
                showOwner={false}
              />
              <LockSection
                icon={<Users className="size-4" />}
                title="他人のロック"
                locks={report.theirs}
                showOwner
              />
            </div>
          )}
        </div>
      </main>
    </div>
  );
}

function Welcome({ onOpen }: { onOpen: () => void }) {
  return (
    <div className="flex flex-col items-center gap-4 rounded-xl border border-dashed border-zinc-300 px-6 py-16 text-center dark:border-zinc-700">
      <FolderGit2 className="size-10 text-zinc-400" />
      <div>
        <div className="font-semibold">リポジトリのフォルダを開いてください</div>
        <div className="mt-1 text-sm text-zinc-500">Git LFS のロックの一覧を表示します</div>
      </div>
      <Button onClick={onOpen}>
        <FolderOpen />
        フォルダを開く
      </Button>
    </div>
  );
}

function ErrorBox({ message }: { message: string }) {
  return (
    <div className="flex gap-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-900 dark:border-red-500/30 dark:bg-red-500/10 dark:text-red-200">
      <AlertTriangle className="mt-0.5 size-4 shrink-0" />
      <div className="break-words select-text">{message}</div>
    </div>
  );
}

function Loading() {
  return (
    <div className="flex items-center justify-center gap-2 py-16 text-sm text-zinc-500">
      <RefreshCw className="size-4 animate-spin" />
      ロックの一覧を取得しています…
    </div>
  );
}

function LockSection({
  icon,
  title,
  locks,
  showOwner,
}: {
  icon: ReactNode;
  title: string;
  locks: LockRow[];
  showOwner: boolean;
}) {
  return (
    <section className="overflow-hidden rounded-xl border border-zinc-200 bg-white shadow-xs dark:border-zinc-800 dark:bg-zinc-900">
      <h2 className="flex items-center gap-2 border-b border-zinc-100 px-4 py-2.5 text-sm font-semibold dark:border-zinc-800">
        <span className="text-zinc-500">{icon}</span>
        {title}
        <Badge>{locks.length}</Badge>
      </h2>
      {locks.length === 0 ? (
        <div className="px-4 py-6 text-center text-sm text-zinc-500">なし</div>
      ) : (
        <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
          {locks.map((lock) => (
            <LockItem key={lock.id} lock={lock} showOwner={showOwner} />
          ))}
        </ul>
      )}
    </section>
  );
}

function LockItem({ lock, showOwner }: { lock: LockRow; showOwner: boolean }) {
  const slash = lock.path.lastIndexOf("/");
  const folder = lock.path.slice(0, slash + 1);
  const file = lock.path.slice(slash + 1);
  return (
    <li className="flex items-center gap-4 px-4 py-2.5 text-sm hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
      <div className="min-w-0 flex-1 truncate font-mono text-[13px] select-text" title={lock.path}>
        <span className="text-zinc-400 dark:text-zinc-500">{folder}</span>
        <span className="font-medium">{file}</span>
      </div>
      {showOwner && <div className="w-32 shrink-0 truncate text-zinc-600 dark:text-zinc-300">{lock.owner}</div>}
      <div className="shrink-0 text-right whitespace-nowrap text-zinc-500 tabular-nums">{lock.lockedAt} から</div>
      <div className="flex w-28 shrink-0 justify-end">
        {lock.stale ? (
          <Badge variant="warning" title="古いロック（解除し忘れていないか確かめてください）">
            <AlertTriangle />
            {lock.age}
          </Badge>
        ) : (
          <span className="text-zinc-500 tabular-nums">{lock.age}</span>
        )}
      </div>
    </li>
  );
}
