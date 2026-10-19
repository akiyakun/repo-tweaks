import { AlertTriangle, LockOpen, ShieldAlert } from "lucide-react";
import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { LockRow } from "@/lib/api";

type Action = { label: string; danger?: boolean; onClick: (lock: LockRow) => void };

export function LockSection({
  icon,
  title,
  locks,
  showOwner,
  action,
  disabled,
}: {
  icon: ReactNode;
  title: string;
  locks: LockRow[];
  showOwner: boolean;
  action: Action;
  disabled: boolean;
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
            <LockItem key={lock.id} lock={lock} showOwner={showOwner} action={action} disabled={disabled} />
          ))}
        </ul>
      )}
    </section>
  );
}

function LockItem({
  lock,
  showOwner,
  action,
  disabled,
}: {
  lock: LockRow;
  showOwner: boolean;
  action: Action;
  disabled: boolean;
}) {
  const slash = lock.path.lastIndexOf("/");
  const folder = lock.path.slice(0, slash + 1);
  const file = lock.path.slice(slash + 1);
  return (
    <li className="group flex items-center gap-4 py-2 pr-2 pl-4 text-sm hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
      <div className="min-w-0 flex-1 truncate font-mono text-[13px] select-text" title={lock.path}>
        <span className="text-zinc-400 dark:text-zinc-500">{folder}</span>
        <span className="font-medium">{file}</span>
      </div>
      {showOwner && <div className="w-32 shrink-0 truncate text-zinc-600 dark:text-zinc-300">{lock.owner}</div>}
      <div className="shrink-0 text-right whitespace-nowrap text-zinc-500 tabular-nums">{lock.lockedAt} から</div>
      <div className="flex w-24 shrink-0 justify-end">
        {lock.stale ? (
          <Badge variant="warning" title="古いロック（解除し忘れていないか確かめてください）">
            <AlertTriangle />
            {lock.age}
          </Badge>
        ) : (
          <span className="text-zinc-500 tabular-nums">{lock.age}</span>
        )}
      </div>
      <Button
        variant="ghost"
        size="sm"
        className={
          action.danger
            ? "w-24 text-red-600 hover:bg-red-50 hover:text-red-700 dark:text-red-400 dark:hover:bg-red-500/10"
            : "w-24"
        }
        onClick={() => action.onClick(lock)}
        disabled={disabled}
      >
        {action.danger ? <ShieldAlert /> : <LockOpen />}
        {action.label}
      </Button>
    </li>
  );
}
