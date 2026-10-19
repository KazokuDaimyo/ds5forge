// Optional components (ViGEmBus, HidHide): their status and a button to install them.
//
// The installation itself is done by the backend through winget (see src-tauri/src/install.rs):
// nothing is downloaded or run from here, and the user confirms each installation.

import { invoke } from "@tauri-apps/api/core";
import { t, type Key } from "./i18n";
import { $ } from "./ui-common";
import { onViewChange } from "./views";

// Mirror of `commands::ComponentsStatus` (src-tauri/src/commands.rs).
interface Status {
  vigembus: boolean;
  hidhide: boolean;
  hidhide_ready: boolean;
}

type Outcome = "installed" | "opened-page";

interface Row {
  id: "vigembus" | "hidhide";
  name: string;
  description: Key;
}

const ROWS: Row[] = [
  { id: "vigembus", name: "ViGEmBus", description: "components.vigembus" },
  { id: "hidhide", name: "HidHide", description: "components.hidhide" },
];

export function initComponentsUi() {
  const list = $("components-list");
  const message = $("components-msg");
  const defaultMessage = message.textContent ?? "";
  /** Components being installed: their button stays disabled across re-renders. */
  const busy = new Set<Row["id"]>();

  const say = (text: string, error = false) => {
    message.textContent = text;
    message.classList.toggle("error", error);
  };

  async function install(row: Row) {
    if (!confirm(t("components.confirm", { name: row.name }))) return;
    busy.add(row.id);
    say(defaultMessage);
    await render();
    try {
      const outcome = await invoke<Outcome>("install_component", { component: row.id });
      if (outcome === "opened-page") say(t("components.openedPage"));
      else say(t(row.id === "hidhide" ? "components.doneRestart" : "components.done", { name: row.name }));
    } catch (error) {
      say(String(error), true);
    } finally {
      busy.delete(row.id);
      await render();
    }
  }

  async function render() {
    let status: Status;
    try {
      status = await invoke<Status>("components_status");
    } catch {
      return;
    }
    list.replaceChildren(
      ...ROWS.map((row) => {
        const installed = row.id === "vigembus" ? status.vigembus : status.hidhide;
        const needsRestart = row.id === "hidhide" && status.hidhide && !status.hidhide_ready;

        const item = document.createElement("div");
        item.className = "component";

        const info = document.createElement("div");
        info.className = "component-info";
        const name = document.createElement("span");
        name.className = "component-name";
        name.textContent = row.name;
        const description = document.createElement("span");
        description.className = "hint";
        description.textContent = t(row.description);
        info.append(name, description);

        const pill = document.createElement("span");
        pill.className = `status ${installed && !needsRestart ? "on" : "off"}`;
        pill.textContent = needsRestart
          ? t("components.needsRestart")
          : installed
            ? t("components.installed")
            : t("components.missing");

        item.append(info, pill);

        if (needsRestart) {
          const restart = document.createElement("button");
          restart.type = "button";
          restart.textContent = t("components.restart");
          restart.addEventListener("click", () => invoke("restart_app"));
          item.append(restart);
        } else if (!installed) {
          const button = document.createElement("button");
          button.type = "button";
          button.disabled = busy.has(row.id);
          button.textContent = busy.has(row.id) ? t("components.installing") : t("components.install");
          button.addEventListener("click", () => void install(row));
          item.append(button);
        }
        return item;
      }),
    );
  }

  // The status is read whenever it may have changed: when the page opens, and when the window gets
  // the focus back (the user may have installed a component by other means).
  onViewChange((view) => {
    if (view === "settings") void render();
  });
  window.addEventListener("focus", () => void render());
  void render();
}
