// Diagnostics screen: state of the drivers, of the rights and of each controller, and the error log.

import { invoke } from "@tauri-apps/api/core";
import { chargeMark, rangeText, type Charge } from "./battery";
import { locale, t } from "./i18n";
import { $ } from "./ui-common";
import { getView, onViewChange, type View } from "./views";

// Mirror of `diagnostics::Report` (src-tauri/src/diagnostics.rs).
interface Entry {
  time_ms: number;
  level: "info" | "warn" | "error";
  /** Stable identifier of the origin ("controller", "hidhide", "udp"…), shown as is. */
  source: string;
  message: string;
  count: number;
}

interface ControllerDiag {
  id: string;
  name: string;
  connected: boolean;
  bluetooth: boolean;
  path: string | null;
  battery: number | null;
  charge: Charge;
  reports_per_second: number;
  virtual_pad: boolean;
  hidden: boolean;
}

interface Report {
  app: { version: string; exe: string | null; config_dir: string | null; elevated: boolean };
  vigem: { ok: boolean; message: string | null };
  hidhide: { installed: boolean; cloak_active: boolean | null; app_registered: boolean | null; error: string | null };
  settings: { hidhide_auto: boolean; low_battery_alert: boolean; haptic_feedback: boolean; language: string | null };
  controllers: ControllerDiag[];
  udp: { listening: boolean; port: number; error: string | null };
  log: Entry[];
}

const yesNo = (v: boolean) => (v ? t("diag.yes") : t("diag.no"));
const time = (ms: number) => new Date(ms).toLocaleTimeString(locale);

function hidhideLine(h: Report["hidhide"]): string {
  if (!h.installed) return t("diag.hidhideNotInstalled");
  if (h.error) return t("diag.hidhideError", { error: h.error });
  return t("diag.hidhideInstalled", {
    cloak: h.cloak_active ? t("diag.cloakActive") : t("diag.cloakInactive"),
    app: h.app_registered ? t("diag.appRegistered") : t("diag.appNotRegistered"),
  });
}

function controllerLine(c: ControllerDiag): string {
  const parts = [c.bluetooth && c.connected ? "Bluetooth" : c.connected ? "USB" : t("diag.offline")];
  if (c.connected) {
    parts.push(t("diag.reportsPerSecond", { n: c.reports_per_second }));
    if (c.battery !== null) parts.push(t("diag.batteryInfo", { range: rangeText(c.battery) + chargeMark(c.charge) }));
    parts.push(c.virtual_pad ? t("diag.modeXbox") : t("diag.modeNative"));
    if (c.hidden) parts.push(t("diag.hiddenByHidHide"));
  }
  return `  ${c.name || t("diag.unnamed")} [${c.id}] — ${parts.join(", ")}`;
}

/** The report as plain text: what is displayed, and what is copied to ask for help. */
export function reportToText(r: Report): string {
  const unknown = t("diag.unknown");
  const lines = [
    `DS5Forge ${r.app.version}`,
    t("diag.executable", { value: r.app.exe ?? unknown }),
    t("diag.configDir", { value: r.app.config_dir ?? unknown }),
    t("diag.admin", { value: yesNo(r.app.elevated) }),
    "",
    t("diag.drivers"),
    `  ViGEmBus: ${r.vigem.ok ? t("diag.vigemOk") : t("diag.vigemProblem", { message: r.vigem.message ?? unknown })}`,
    `  HidHide: ${hidhideLine(r.hidhide)}`,
    r.udp.listening
      ? t("diag.udpListening", { port: r.udp.port })
      : t("diag.udpStopped", { error: r.udp.error ?? "?" }),
    "",
    t("diag.controllers", { count: r.controllers.length }),
    ...(r.controllers.length ? r.controllers.map(controllerLine) : [`  ${t("diag.none")}`]),
    "",
    t("diag.settings", {
      hidhide: yesNo(r.settings.hidhide_auto),
      battery: yesNo(r.settings.low_battery_alert),
      haptics: yesNo(r.settings.haptic_feedback),
      language: r.settings.language ?? "auto",
    }),
    "",
    t("diag.log", { count: r.log.length }),
    ...(r.log.length
      ? r.log.map((e) => {
          const repeat = e.count > 1 ? `  (×${e.count})` : "";
          return `  ${time(e.time_ms)}  ${e.level.toUpperCase().padEnd(5)}  ${e.source}  ${e.message}${repeat}`;
        })
      : [`  ${t("diag.empty")}`]),
  ];
  return lines.join("\n");
}

function pill(text: string, ok: boolean): HTMLElement {
  const el = document.createElement("span");
  el.className = `status ${ok ? "on" : "bad"}`;
  el.textContent = text;
  return el;
}

export function initDiagnosticsUi() {
  const pills = $("diag-pills");
  const text = $("diag-text");
  const copyButton = $<HTMLButtonElement>("diag-copy");
  let last = "";
  let timer: number | undefined;

  async function refresh() {
    try {
      const report = await invoke<Report>("get_diagnostics");
      last = reportToText(report);
      text.textContent = last;
      pills.replaceChildren(
        pill(report.vigem.ok ? "ViGEmBus ✓" : "ViGEmBus ✗", report.vigem.ok),
        pill(
          !report.hidhide.installed ? t("diag.pillHidhideAbsent") : report.hidhide.error ? "HidHide ✗" : "HidHide ✓",
          report.hidhide.installed && !report.hidhide.error,
        ),
        pill(report.udp.listening ? "UDP ✓" : "UDP ✗", report.udp.listening),
        pill(report.app.elevated ? t("diag.pillAdmin") : t("diag.pillNormal"), true),
      );
    } catch (error) {
      text.textContent = t("diag.unavailable", { error: String(error) });
    }
  }

  // Automatic refresh while the page is displayed.
  const sync = (view: View) => {
    window.clearInterval(timer);
    if (view === "diagnostic") {
      refresh();
      timer = window.setInterval(refresh, 3000);
    }
  };
  onViewChange(sync);
  sync(getView());

  $("diag-refresh").addEventListener("click", refresh);

  copyButton.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(last);
      copyButton.textContent = t("diag.copied");
    } catch {
      copyButton.textContent = t("diag.copyFailed");
    }
    window.setTimeout(() => (copyButton.textContent = t("diag.copy")), 1500);
  });

  $("diag-clear").addEventListener("click", async () => {
    await invoke("clear_log");
    refresh();
  });
}
