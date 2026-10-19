import { useEffect, useRef } from "react";
import { Button } from "@/components/ui/button";

export type Confirm = {
  title: string;
  message: string;
  items: { path: string; detail?: string }[];
  confirmLabel: string;
  onConfirm: () => void;
};

/** A modal question before a change that is hard to undo. Escape or the backdrop cancels. */
export function ConfirmDialog({ confirm, onCancel }: { confirm: Confirm; onCancel: () => void }) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    // Start on "cancel", so Enter alone does not remove anyone's lock
    cancelRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6" onClick={onCancel}>
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        className="w-full max-w-lg rounded-xl border border-zinc-200 bg-white p-5 shadow-xl dark:border-zinc-800 dark:bg-zinc-900"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id="confirm-title" className="font-semibold">
          {confirm.title}
        </h2>
        <p className="mt-2 text-sm text-zinc-600 dark:text-zinc-300">{confirm.message}</p>
        <ul className="mt-3 max-h-48 overflow-y-auto rounded-lg bg-zinc-50 px-3 py-2 text-sm dark:bg-zinc-800/60">
          {confirm.items.map((item) => (
            <li key={item.path} className="flex flex-wrap gap-x-3 py-0.5">
              <span className="font-mono text-[13px] break-all">{item.path}</span>
              {item.detail && <span className="text-zinc-500">{item.detail}</span>}
            </li>
          ))}
        </ul>
        <div className="mt-5 flex justify-end gap-2">
          <Button ref={cancelRef} variant="outline" onClick={onCancel}>
            キャンセル
          </Button>
          <Button
            className="bg-red-600 text-white hover:bg-red-700 dark:bg-red-600 dark:text-white dark:hover:bg-red-700"
            onClick={confirm.onConfirm}
          >
            {confirm.confirmLabel}
          </Button>
        </div>
      </div>
    </div>
  );
}
