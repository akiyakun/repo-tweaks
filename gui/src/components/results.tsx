import { CheckCircle2, CircleSlash, X, XCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { OpResult } from "@/lib/api";
import { cn } from "@/lib/utils";

export type Results = { title: string; rows: OpResult[] } | { title: string; error: string };

/** What happened to each file in the last lock / unlock, worded like the CLI. */
export function ResultsPanel({
  results,
  onClose,
  onForce,
  disabled,
}: {
  results: Results;
  onClose: () => void;
  onForce: (path: string) => void;
  disabled: boolean;
}) {
  const rows = "rows" in results ? results.rows : [];
  const failed = rows.filter((r) => !r.ok).length;
  const bad = "error" in results || failed > 0;
  return (
    <section
      className={cn(
        "overflow-hidden rounded-xl border bg-white shadow-xs dark:bg-zinc-900",
        bad ? "border-red-200 dark:border-red-500/30" : "border-emerald-200 dark:border-emerald-500/30",
      )}
    >
      <h2 className="flex items-center gap-2 border-b border-zinc-100 py-1.5 pr-1.5 pl-4 text-sm font-semibold dark:border-zinc-800">
        {bad ? <XCircle className="size-4 text-red-500" /> : <CheckCircle2 className="size-4 text-emerald-500" />}
        <span className="flex-1">
          {results.title}
          {failed > 0 && <span className="ml-2 font-normal text-red-600 dark:text-red-400">{failed} 件は処理できませんでした</span>}
        </span>
        <Button variant="ghost" size="icon" onClick={onClose} aria-label="閉じる" title="閉じる">
          <X />
        </Button>
      </h2>
      {"error" in results ? (
        <div className="px-4 py-3 text-sm break-words text-red-700 select-text dark:text-red-300">{results.error}</div>
      ) : (
        <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
          {rows.map((row, i) => (
            <li key={`${row.path}-${i}`} className="flex items-start gap-3 px-4 py-2 text-sm">
              <ResultIcon row={row} />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-baseline gap-x-3">
                  <span className="font-medium">{row.label}</span>
                  <span className="font-mono text-[13px] break-all select-text">
                    {row.path}
                    {row.autoAdded && <span className="font-sans text-zinc-500">（自動で追加）</span>}
                  </span>
                </div>
                {row.detail && <div className="mt-0.5 break-words text-zinc-500 select-text">{row.detail}</div>}
              </div>
              {row.canForce && (
                <Button variant="outline" size="sm" onClick={() => onForce(row.path)} disabled={disabled}>
                  変更を残したまま解除
                </Button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function ResultIcon({ row }: { row: OpResult }) {
  const className = "mt-0.5 size-4 shrink-0";
  if (!row.ok) return <XCircle className={cn(className, "text-red-500")} />;
  // "ロック済み", "ロックなし", "しませんでした": nothing changed, but nothing went wrong either
  const changed = /しました$/.test(row.label);
  return changed ? (
    <CheckCircle2 className={cn(className, "text-emerald-500")} />
  ) : (
    <CircleSlash className={cn(className, "text-zinc-400")} />
  );
}
