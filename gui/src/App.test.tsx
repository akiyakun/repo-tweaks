import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode } from "react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { callsTo, dropPaths, fail, fakeBackend, lock, opReport, report, result } from "./test/backend";

const LAST_FOLDER_KEY = "repo-tweaks.lastFolder";

// As in main.tsx: StrictMode runs effects twice in development, which the app must survive
function renderApp() {
  return render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

describe("start-up", () => {
  it("shows the welcome screen when no folder is known", async () => {
    const calls = fakeBackend({ initial_folder: () => null });
    renderApp();
    expect(await screen.findByText("リポジトリのフォルダを開いてください")).toBeInTheDocument();
    expect(callsTo(calls, "list_locks")).toEqual([]);
  });

  it("opens the command-line folder once, before the remembered one", async () => {
    localStorage.setItem(LAST_FOLDER_KEY, "/work/old");
    const calls = fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () => report({ ours: [lock("Assets/Main.unity", "alice")] }),
    });
    renderApp();
    expect(await screen.findByText("Main.unity")).toBeInTheDocument();
    // Once, despite StrictMode, and never for the remembered folder
    expect(callsTo(calls, "initial_folder")).toHaveLength(1);
    expect(callsTo(calls, "list_locks")).toEqual([{ dir: "/work/game" }]);
    expect(localStorage.getItem(LAST_FOLDER_KEY)).toBe("/work/game");
  });

  it("falls back to the remembered folder", async () => {
    localStorage.setItem(LAST_FOLDER_KEY, "/work/game");
    const calls = fakeBackend({ initial_folder: () => null, list_locks: () => report() });
    renderApp();
    expect(await screen.findByText("10/20 15:00 に更新")).toBeInTheDocument();
    expect(callsTo(calls, "list_locks")).toEqual([{ dir: "/work/game" }]);
  });

  it("does not remember a folder that failed to open", async () => {
    fakeBackend({
      initial_folder: () => "/work/typo",
      list_locks: () => fail("フォルダが見つかりません: /work/typo"),
    });
    renderApp();
    expect(await screen.findByText("フォルダが見つかりません: /work/typo")).toBeInTheDocument();
    expect(localStorage.getItem(LAST_FOLDER_KEY)).toBeNull();
  });
});

describe("lock list", () => {
  it("shows both sections, the stale warning and the owner of other people's locks", async () => {
    fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () =>
        report({
          staleWarning: "3日以上ロックされたままのものが 1 件あります",
          ours: [lock("Assets/Main.unity", "alice")],
          theirs: [lock("Assets/Art/hero.psd", "bob", { stale: true, age: "4日17時間" })],
        }),
    });
    renderApp();
    expect(await screen.findByText("3日以上ロックされたままのものが 1 件あります")).toBeInTheDocument();
    const theirs = screen.getByText("他人のロック").closest("section")!;
    expect(within(theirs).getByText("bob")).toBeInTheDocument();
    expect(within(theirs).getByText("4日17時間")).toBeInTheDocument();
    expect(within(theirs).getByRole("button", { name: /強制解除/ })).toBeInTheDocument();
    const ours = screen.getByText("自分のロック").closest("section")!;
    expect(within(ours).getByRole("button", { name: /解除/ })).toBeInTheDocument();
  });

  it("keeps the last list when a refresh fails", async () => {
    let fetches = 0;
    fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () =>
        ++fetches === 1 ? report({ ours: [lock("Assets/Main.unity", "alice")] }) : fail("サーバーにつながりませんでした"),
    });
    renderApp();
    await screen.findByText("Main.unity");
    await userEvent.click(screen.getByRole("button", { name: "更新" }));
    expect(await screen.findByText("サーバーにつながりませんでした")).toBeInTheDocument();
    expect(screen.getByText("10/20 15:00 に取得した一覧を表示しています")).toBeInTheDocument();
    expect(screen.getByText("Main.unity")).toBeInTheDocument();
  });
});

describe("unlock", () => {
  it("unlocks our own lock and shows the results with the notification", async () => {
    const calls = fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () => report({ ours: [lock("Assets/Main.unity", "alice")] }),
      unlock_files: () =>
        opReport(
          [result("Assets/Main.unity", "解除しました"), result("Assets/Main.unity.meta", "解除しました", { autoAdded: true })],
          { kind: "sent", message: "サーバーに通知しました" },
        ),
    });
    renderApp();
    await screen.findByText("Main.unity");
    await userEvent.click(screen.getByRole("button", { name: /解除/ }));
    expect(await screen.findByText("サーバーに通知しました")).toBeInTheDocument();
    expect(screen.getByText("（自動で追加）")).toBeInTheDocument();
    expect(callsTo(calls, "unlock_files")).toEqual([
      { root: "/work/game", paths: ["Assets/Main.unity"], force: false, confirmed: [] },
    ]);
    // The list is fetched again after the change
    await waitFor(() => expect(callsTo(calls, "list_locks")).toHaveLength(2));
  });

  it("offers to keep the changes when git lfs refuses our own lock, after asking", async () => {
    const calls = fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () => report({ ours: [lock("Assets/Main.unity", "alice")] }),
      unlock_files: (args) =>
        args.force
          ? opReport([result("Assets/Main.unity", "解除しました")])
          : opReport([
              result("Assets/Main.unity", "できません", {
                ok: false,
                canForce: true,
                detail: "未コミットの変更があるため解除できません。",
              }),
            ]),
    });
    renderApp();
    await screen.findByText("Main.unity");
    await userEvent.click(screen.getByRole("button", { name: /解除/ }));
    await userEvent.click(await screen.findByRole("button", { name: "変更を残したまま解除" }));
    const dialog = screen.getByRole("alertdialog");
    expect(within(dialog).getByText("Assets/Main.unity")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "解除する" }));
    expect(await screen.findByText("解除しました")).toBeInTheDocument();
    expect(callsTo(calls, "unlock_files").map((a) => a.force)).toEqual([false, true]);
    expect(callsTo(calls, "unlock_files")[1]).toEqual({
      root: "/work/game",
      paths: ["Assets/Main.unity"],
      force: true,
      confirmed: [],
    });
  });
});

describe("forced unlock of someone else's lock", () => {
  const backend = () =>
    fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () => report({ theirs: [lock("Assets/Art/hero.psd", "bob")] }),
      preview_force_unlock: () => [
        { id: "3", path: "Assets/Art/hero.psd", owner: "bob", lockedAt: "10/15 20:01" },
        { id: "4", path: "Assets/Art/hero.psd.meta", owner: "bob", lockedAt: "10/15 20:01" },
      ],
      unlock_files: () => opReport([result("Assets/Art/hero.psd", "強制解除しました", { detail: "bob さんのロック" })]),
    });

  it("lists every lock that would go, and does nothing on cancel", async () => {
    const calls = backend();
    renderApp();
    await screen.findByText("hero.psd");
    await userEvent.click(screen.getByRole("button", { name: /強制解除/ }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("Assets/Art/hero.psd")).toBeInTheDocument();
    expect(within(dialog).getByText("Assets/Art/hero.psd.meta")).toBeInTheDocument();
    // Focus starts on cancel, so Enter alone removes nobody's lock
    expect(within(dialog).getByRole("button", { name: "キャンセル" })).toHaveFocus();
    await userEvent.click(within(dialog).getByRole("button", { name: "キャンセル" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(callsTo(calls, "unlock_files")).toEqual([]);
  });

  it("closes on Escape without unlocking", async () => {
    const calls = backend();
    renderApp();
    await screen.findByText("hero.psd");
    await userEvent.click(screen.getByRole("button", { name: /強制解除/ }));
    await screen.findByRole("alertdialog");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(callsTo(calls, "unlock_files")).toEqual([]);
  });

  it("passes exactly the confirmed lock IDs", async () => {
    const calls = backend();
    renderApp();
    await screen.findByText("hero.psd");
    await userEvent.click(screen.getByRole("button", { name: /強制解除/ }));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "強制解除する" }));
    expect(await screen.findByText("強制解除しました")).toBeInTheDocument();
    expect(callsTo(calls, "unlock_files")).toEqual([
      { root: "/work/game", paths: ["Assets/Art/hero.psd"], force: true, confirmed: ["3", "4"] },
    ]);
  });

  it("says so when the lock is already gone", async () => {
    const calls = fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () => report({ theirs: [lock("Assets/Art/hero.psd", "bob")] }),
      preview_force_unlock: () => [],
    });
    renderApp();
    await screen.findByText("hero.psd");
    await userEvent.click(screen.getByRole("button", { name: /強制解除/ }));
    expect(await screen.findByText("Assets/Art/hero.psd のロックは、すでに解除されています")).toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(callsTo(calls, "unlock_files")).toEqual([]);
  });

  it("shows a refusal from the Rust side instead of results", async () => {
    fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () => report({ theirs: [lock("Assets/Art/hero.psd", "bob")] }),
      preview_force_unlock: () => [{ id: "3", path: "Assets/Art/hero.psd", owner: "bob", lockedAt: "10/15 20:01" }],
      unlock_files: () => fail("確認したあとにロックの状態が変わりました。一覧を更新して、もう一度やり直してください（何も変更していません）"),
    });
    renderApp();
    await screen.findByText("hero.psd");
    await userEvent.click(screen.getByRole("button", { name: /強制解除/ }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "強制解除する" }));
    expect(await screen.findByText(/確認したあとにロックの状態が変わりました/)).toBeInTheDocument();
  });
});

describe("lock", () => {
  it("locks the files picked in the dialog, starting in the repository", async () => {
    const calls = fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () => report(),
      "plugin:dialog|open": () => ["/work/game/Assets/Main.unity"],
      lock_files: () =>
        opReport([result("Assets/Main.unity", "ロックしました")], {
          kind: "info",
          message: "通知の送り先（repo-tweaks.server）が未設定なので、Discord には送っていません",
        }),
    });
    renderApp();
    await screen.findByText("10/20 15:00 に更新");
    await userEvent.click(screen.getByRole("button", { name: /ファイルをロック/ }));
    expect(await screen.findByText("ロックしました")).toBeInTheDocument();
    expect(screen.getByText(/Discord には送っていません/)).toBeInTheDocument();
    const dialog = callsTo(calls, "plugin:dialog|open")[0].options as Record<string, unknown>;
    expect(dialog.multiple).toBe(true);
    expect(dialog.defaultPath).toBe("/work/game");
    expect(callsTo(calls, "lock_files")).toEqual([{ root: "/work/game", paths: ["/work/game/Assets/Main.unity"] }]);
  });

  it("does nothing when the dialog is cancelled", async () => {
    const calls = fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () => report(),
      "plugin:dialog|open": () => null,
    });
    renderApp();
    await screen.findByText("10/20 15:00 に更新");
    await userEvent.click(screen.getByRole("button", { name: /ファイルをロック/ }));
    await waitFor(() => expect(callsTo(calls, "plugin:dialog|open")).toHaveLength(1));
    expect(callsTo(calls, "lock_files")).toEqual([]);
  });

  it("shows a notification that could not be sent as a warning", async () => {
    fakeBackend({
      initial_folder: () => "/work/game",
      list_locks: () => report(),
      "plugin:dialog|open": () => ["/work/game/a.psd"],
      lock_files: () =>
        opReport([result("a.psd", "ロックしました")], {
          kind: "warning",
          message: "通知を送れませんでした（ロック・解除は済んでいます）: サーバーにつながりません",
        }),
    });
    renderApp();
    await screen.findByText("10/20 15:00 に更新");
    await userEvent.click(screen.getByRole("button", { name: /ファイルをロック/ }));
    expect(await screen.findByText(/通知を送れませんでした/)).toBeInTheDocument();
  });
});

describe("drag and drop", () => {
  it("opens a dropped folder", async () => {
    const calls = fakeBackend({
      initial_folder: () => null,
      dropped_folder: () => "/work/other",
      list_locks: () => report({ root: "/work/other", name: "other" }),
    });
    renderApp();
    await screen.findByText("リポジトリのフォルダを開いてください");
    await dropPaths(["/work/other"]);
    expect(await screen.findByText("other")).toBeInTheDocument();
    expect(callsTo(calls, "dropped_folder")).toEqual([{ paths: ["/work/other"] }]);
    expect(callsTo(calls, "list_locks")).toEqual([{ dir: "/work/other" }]);
  });

  it("explains when only files are dropped", async () => {
    const calls = fakeBackend({ initial_folder: () => null, dropped_folder: () => null });
    renderApp();
    await screen.findByText("リポジトリのフォルダを開いてください");
    await dropPaths(["/work/game/a.psd"]);
    expect(await screen.findByText(/フォルダをドロップしてください/)).toBeInTheDocument();
    expect(callsTo(calls, "list_locks")).toEqual([]);
  });
});
