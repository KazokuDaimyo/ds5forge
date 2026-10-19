// Game profiles page: a game profile has its own settings and is used automatically while its
// program is in the foreground (see src-tauri/src/games.rs). The user's usual settings are the default.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t } from "./i18n";
import { icon } from "./icons";
import type { PresetsInfo } from "./types";
import { $ } from "./ui-common";
import { onViewChange } from "./views";

const MAX_NAME_LEN = 32;

/**
 * A readable name for a program: "TheCrewMotorfest.exe" becomes "The Crew Motorfest". The extension
 * goes, words glued in camel case are split, and so are digits ("ForzaHorizon5" gives "Forza Horizon 5").
 */
export function readableProgramName(exe: string): string {
  return exe
    .replace(/\.exe$/i, "")
    .replace(/[_-]+/g, " ")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
    .replace(/([A-Za-z])(\d)/g, "$1 $2")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * `defaultProfile` is the default profile of the selected controller: a new game profile starts as a
 * copy of it. `edit` shows a game profile on the selected controller, to edit it.
 */
export function initProfilesUi(defaultProfile: () => string | null, edit: (name: string) => void) {
  const gamesEl = $("games-list");
  const gamesMsg = $("games-msg");
  const addMsg = $("add-msg");
  const runningSelect = $<HTMLSelectElement>("game-running");
  const exeInput = $<HTMLInputElement>("game-exe");
  const nameInput = $<HTMLInputElement>("game-name");

  let info: PresetsInfo = { presets: [], games: [] };

  const say = (el: HTMLElement, text: string, error = false) => {
    el.textContent = text;
    el.classList.toggle("error", error);
  };

  async function load() {
    info = await invoke<PresetsInfo>("get_presets");
    render();
  }

  /** Runs a backend command; shows its error under the list it belongs to, then reloads. */
  async function run(msg: HTMLElement, command: string, args: Record<string, unknown>) {
    say(msg, "");
    try {
      await invoke(command, args);
    } catch (error) {
      say(msg, String(error), true);
    }
    await load();
  }

  function button(label: string, onClick: () => void, className = ""): HTMLButtonElement {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = label;
    if (className) b.className = className;
    b.addEventListener("click", onClick);
    return b;
  }

  function render() {
    if (info.games.length === 0) {
      const empty = document.createElement("p");
      empty.className = "hint";
      empty.textContent = t("profiles.noGames");
      gamesEl.replaceChildren(empty);
      return;
    }
    gamesEl.replaceChildren(
      ...info.games.map((game) => {
        const row = document.createElement("div");
        row.className = "preset-row";

        const name = document.createElement("input");
        name.type = "text";
        name.maxLength = MAX_NAME_LEN;
        name.value = game.preset;
        name.addEventListener("change", () => {
          if (name.value.trim() !== game.preset) void run(gamesMsg, "rename_preset", { old: game.preset, new: name.value });
        });

        // The program, in plain words; the exact file name is in the tooltip.
        const exe = document.createElement("span");
        exe.className = "program";
        exe.textContent = readableProgramName(game.exe);
        exe.title = game.exe;

        const remove = button(
          "",
          () => {
            if (confirm(t("profiles.confirmDelete", { name: game.preset }))) {
              void run(gamesMsg, "delete_preset", { name: game.preset });
            }
          },
          "icon-btn danger",
        );
        remove.innerHTML = icon("close", 15);
        remove.title = t("profiles.delete");
        remove.setAttribute("aria-label", t("profiles.delete"));

        row.append(name, exe, button(t("profiles.edit"), () => edit(game.preset)), remove);
        return row;
      }),
    );
  }

  /** The programs with a window, to pick a game from. */
  async function loadRunningApps() {
    let apps: string[] = [];
    try {
      apps = await invoke<string[]>("list_running_apps");
    } catch {
      // The list is only a convenience: the name can still be typed.
    }
    runningSelect.replaceChildren(
      new Option(t("profiles.addRunning"), ""),
      ...apps.map((a) => new Option(readableProgramName(a) + " (" + a + ")", a)),
    );
  }

  /** A profile name that is free: the name itself, then "name 2", "name 3"… (the backend checks it again). */
  function freeName(base: string): string {
    const stem = base.slice(0, MAX_NAME_LEN);
    let candidate = stem;
    for (let n = 2; info.presets.includes(candidate); n++) {
      const suffix = " " + n;
      candidate = stem.slice(0, MAX_NAME_LEN - suffix.length) + suffix;
    }
    return candidate;
  }

  runningSelect.addEventListener("change", () => {
    if (runningSelect.value) exeInput.value = runningSelect.value;
  });

  $("game-create").addEventListener("click", async () => {
    const exe = exeInput.value.trim();
    const from = defaultProfile();
    if (!exe || !from) return;
    // Without a name, the profile takes the name of the program.
    const name = freeName(nameInput.value.trim() || readableProgramName(exe));
    say(addMsg, "");
    try {
      const created = await invoke<string>("create_game_profile", { name, exe, from });
      exeInput.value = "";
      nameInput.value = "";
      runningSelect.value = "";
      await load();
      edit(created); // the new profile is shown right away, ready to be set up
    } catch (error) {
      say(addMsg, String(error), true);
    }
  });

  onViewChange((view) => {
    if (view === "profiles") {
      void load();
      void loadRunningApps();
    }
  });
  void listen("controllers-changed", () => void load());
  void load();
  void loadRunningApps();
}
