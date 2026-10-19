// Updates: looks for a newer version on GitHub, tells the user, and installs it on request.
//
// The check is a single request made by the backend (see src-tauri/src/commands.rs). The download is
// verified against the public key in tauri.conf.json before anything is installed, and the installer
// restarts the application when it is done.

import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { $ } from "./ui-common";

// Mirror of `commands::UpdateInfo`.
interface UpdateInfo {
  current: string;
  available: { version: string; notes: string | null } | null;
}

/**
 * The last answer is kept for the session: changing the language reloads the window, and the button
 * must not disappear until the next check has answered.
 */
const CACHE_KEY = "ds5forge.update";

function cachedAvailable(): UpdateInfo["available"] {
  try {
    return JSON.parse(sessionStorage.getItem(CACHE_KEY) ?? "null");
  } catch {
    return null;
  }
}

function cacheAvailable(available: UpdateInfo["available"]) {
  try {
    sessionStorage.setItem(CACHE_KEY, JSON.stringify(available));
  } catch {
    // Not remembered: the next check will tell.
  }
}

export function initUpdatesUi() {
  const versionEl = $("update-version");
  const message = $("update-msg");
  const checkButton = $<HTMLButtonElement>("update-check");
  const installButton = $<HTMLButtonElement>("update-install");
  const settingsNav = document.querySelector<HTMLElement>('[data-nav="settings"]');
  const defaultMessage = message.textContent ?? "";

  const say = (text: string, error = false) => {
    message.textContent = text;
    message.classList.toggle("error", error);
  };

  /** Shows (or hides) the "available" state: message, install button and dot on the Settings entry. */
  function showAvailable(available: UpdateInfo["available"]) {
    installButton.hidden = !available;
    settingsNav?.classList.toggle("has-update", available !== null);
    if (available) say(t("updates.available", { version: available.version }));
  }

  /** Asks the backend; `quiet` is for the automatic check, which reports nothing unless a version is available. */
  async function check(quiet: boolean) {
    checkButton.disabled = true;
    if (!quiet) say(t("updates.checking"));
    try {
      const info = await invoke<UpdateInfo>("check_update");
      versionEl.textContent = t("updates.version", { version: info.current });
      const available = info.available;
      cacheAvailable(available);
      showAvailable(available);
      if (!available) say(quiet ? defaultMessage : t("updates.upToDate"));
    } catch (error) {
      // The automatic check stays silent when it fails (offline, no release yet).
      if (!quiet) say(t("updates.failed", { error: String(error) }), true);
    } finally {
      checkButton.disabled = false;
    }
  }

  installButton.addEventListener("click", async () => {
    installButton.disabled = true;
    checkButton.disabled = true;
    say(t("updates.installing"));
    try {
      // On success the application exits and the installer starts it again: nothing is left to do here.
      await invoke("install_update");
    } catch (error) {
      say(String(error), true);
      installButton.disabled = false;
      checkButton.disabled = false;
    }
  });

  checkButton.addEventListener("click", () => void check(false));

  // The version is shown from the start.
  void getVersion().then((version) => (versionEl.textContent = t("updates.version", { version })));
  // The check starts right away. What the previous check found is shown meanwhile.
  showAvailable(cachedAvailable());
  void check(true);
}
