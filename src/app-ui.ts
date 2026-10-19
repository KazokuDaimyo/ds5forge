// Settings of the application itself: language, launch at startup, notifications, quit.

import { invoke } from "@tauri-apps/api/core";
import { LANGUAGES, lang, storeLanguage, type Lang } from "./i18n";
import { $ } from "./ui-common";

export function initAppUi() {
  const language = $<HTMLSelectElement>("language");
  const autostart = $<HTMLInputElement>("autostart");
  const lowBattery = $<HTMLInputElement>("opt-low-battery");
  const haptics = $<HTMLInputElement>("opt-haptics");
  const menu = $<HTMLInputElement>("opt-menu");
  const menuFocus = $<HTMLInputElement>("opt-menu-focus");
  const message = $("app-msg");
  const defaultMessage = message.textContent ?? "";

  const showError = (error: unknown) => {
    message.textContent = String(error);
    message.classList.add("error");
  };

  // The backend produces some messages itself (overlay, errors, tray menu): it is told the new
  // language, then the window reloads so that everything is rebuilt in it.
  for (const [code, name] of LANGUAGES) language.add(new Option(name, code));
  language.value = lang;
  language.addEventListener("change", async () => {
    const chosen = language.value as Lang;
    storeLanguage(chosen);
    try {
      await invoke("set_language", { language: chosen });
    } finally {
      window.location.reload();
    }
  });

  // Launch at Windows startup. The real state comes from Windows, not from a file of ours.
  invoke<boolean>("get_autostart")
    .then((enabled) => (autostart.checked = enabled))
    .catch(() => (autostart.disabled = true));

  autostart.addEventListener("change", () => {
    const wanted = autostart.checked;
    message.classList.remove("error");
    message.textContent = defaultMessage;
    invoke("set_autostart", { enabled: wanted }).catch((error) => {
      autostart.checked = !wanted; // the change did not happen
      showError(error);
    });
  });

  // Low-battery alert and confirmation vibration.
  invoke<{ low_battery_alert: boolean; haptic_feedback: boolean; controller_menu: boolean; menu_focus: boolean }>(
    "get_settings",
  ).then((s) => {
    lowBattery.checked = s.low_battery_alert;
    haptics.checked = s.haptic_feedback;
    menu.checked = s.controller_menu;
    menuFocus.checked = s.menu_focus;
    menuFocus.disabled = !s.controller_menu;
  });
  menu.addEventListener("change", () => (menuFocus.disabled = !menu.checked));
  menuFocus.addEventListener("change", () => invoke("set_menu_focus", { enabled: menuFocus.checked }));
  menu.addEventListener("change", () => invoke("set_controller_menu", { enabled: menu.checked }));
  const saveNotifications = () =>
    invoke("set_notifications", { lowBatteryAlert: lowBattery.checked, hapticFeedback: haptics.checked });
  lowBattery.addEventListener("change", saveNotifications);
  haptics.addEventListener("change", saveNotifications);

  $("quit-app").addEventListener("click", () => invoke("quit_app"));
}
