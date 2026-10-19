// Custom title bar: minimize, maximize / restore, close.
// The window has no Windows frame any more; dragging is handled by `data-tauri-drag-region`.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "./i18n";
import { icon } from "./icons";
import { $ } from "./ui-common";

export function initTitlebar() {
  const win = getCurrentWindow();
  const minimize = $<HTMLButtonElement>("win-min");
  const maximize = $<HTMLButtonElement>("win-max");
  const close = $<HTMLButtonElement>("win-close");

  minimize.innerHTML = icon("minus", 14);
  close.innerHTML = icon("close", 14);

  minimize.addEventListener("click", () => win.minimize());
  maximize.addEventListener("click", () => win.toggleMaximize());
  // Closing is intercepted on the Rust side: it minimizes the app to the notification area.
  close.addEventListener("click", () => win.close());

  /** The icon and the tooltip follow the real state of the window (double click, Win+↑…). */
  const syncMaximize = async () => {
    const maximized = await win.isMaximized();
    maximize.innerHTML = icon(maximized ? "restore" : "square", 14);
    maximize.title = maximized ? t("window.restore") : t("window.maximize");
    maximize.setAttribute("aria-label", maximize.title);
  };
  void syncMaximize();
  void win.onResized(syncMaximize);
}
