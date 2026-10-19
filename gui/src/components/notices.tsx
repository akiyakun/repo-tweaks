import { AlertTriangle, FolderGit2, FolderOpen, Info, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";

export function Welcome({ onOpen }: { onOpen: () => void }) {
  return (
    <div className="flex flex-col items-center gap-4 rounded-xl border border-dashed border-zinc-300 px-6 py-16 text-center dark:border-zinc-700">
      <FolderGit2 className="size-10 text-zinc-400" />
      <div>
        <div className="font-semibold">リポジトリのフォルダを開いてください</div>
        <div className="mt-1 text-sm text-zinc-500">
          Git LFS のロックの一覧を表示します。フォルダをこのウィンドウにドロップしても開けます
        </div>
      </div>
      <Button onClick={onOpen}>
        <FolderOpen />
        フォルダを開く
      </Button>
    </div>
  );
}

export function ErrorBox({ message, note }: { message: string; note?: string }) {
  return (
    <div className="flex gap-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-900 dark:border-red-500/30 dark:bg-red-500/10 dark:text-red-200">
      <AlertTriangle className="mt-0.5 size-4 shrink-0" />
      <div className="min-w-0">
        <div className="break-words select-text">{message}</div>
        {note && <div className="mt-1 text-xs text-red-700/80 dark:text-red-300/80">{note}</div>}
      </div>
    </div>
  );
}

export function NoticeBox({ message }: { message: string }) {
  return (
    <div className="flex items-center gap-2 rounded-lg border border-sky-200 bg-sky-50 px-4 py-2.5 text-sm text-sky-900 dark:border-sky-500/30 dark:bg-sky-500/10 dark:text-sky-200">
      <Info className="size-4 shrink-0" />
      {message}
    </div>
  );
}

export function WarningBox({ message }: { message: string }) {
  return (
    <div className="flex items-center gap-2 rounded-lg border border-amber-200 bg-amber-50 px-4 py-2.5 text-sm text-amber-900 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200">
      <AlertTriangle className="size-4 shrink-0" />
      {message}
    </div>
  );
}

export function Loading() {
  return (
    <div className="flex items-center justify-center gap-2 py-16 text-sm text-zinc-500">
      <RefreshCw className="size-4 animate-spin" />
      ロックの一覧を取得しています…
    </div>
  );
}

/** Shown over the window while a folder is dragged onto it. */
export function DropOverlay() {
  return (
    <div className="pointer-events-none fixed inset-0 z-40 flex items-center justify-center bg-white/70 p-6 backdrop-blur-sm dark:bg-zinc-950/70">
      <div className="flex flex-col items-center gap-3 rounded-2xl border-2 border-dashed border-zinc-400 px-12 py-10 text-center dark:border-zinc-500">
        <FolderOpen className="size-10 text-zinc-500" />
        <div className="font-semibold">ドロップしてリポジトリのフォルダを開く</div>
      </div>
    </div>
  );
}
