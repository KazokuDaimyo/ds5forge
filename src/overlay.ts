import { listen } from "@tauri-apps/api/event";
import { chargeMark, createGauge, isLow, rangeText, type Charge } from "./battery";
import { createControllerArt } from "./controller-art";
import { applyTranslations, t } from "./i18n";

// Mirror of OverlayPayload (src-tauri/src/manager.rs).
interface OverlayController {
  id: string;
  name: string;
  color: [number, number, number];
  bluetooth: boolean;
  percent: number | null;
  charge: Charge;
  xbox_mode: boolean;
  source: boolean;
}

interface OverlayPayload {
  message: string | null;
  controllers: OverlayController[];
}

const card = document.getElementById("card")!;
const messageEl = document.getElementById("message")!;
const listEl = document.getElementById("list")!;

function levelText(c: OverlayController): string {
  if (c.charge === "error") return t("overlay.error");
  if (c.percent === null) return "—";
  return rangeText(c.percent) + chargeMark(c.charge);
}

/** A switched-off (black) light bar has no tint: fall back to the accent colour. */
const tint = ([r, g, b]: number[]) => (r + g + b === 0 ? "#5b8cff" : `rgb(${r}, ${g}, ${b})`);

function element<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string) {
  const el = document.createElement(tag);
  el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

function renderItem(c: OverlayController): HTMLLIElement {
  const item = element("li", "item" + (c.source ? " source" : ""));
  item.style.setProperty("--led", tint(c.color));

  // The same illustration as in the app, at rest, with the controller's light bar.
  const art = createControllerArt();
  art.update({ lx: 128, ly: 128, rx: 128, ry: 128, l2: 0, r2: 0, buttons: 0 });
  const thumb = element("div", "thumb");
  thumb.appendChild(art.el);

  const head = element("div", "head");
  const low = c.percent !== null && isLow(c.percent, c.charge);
  head.append(element("span", "name", c.name), element("span", "level" + (low ? " low" : ""), levelText(c)));

  const mode = element(
    "div",
    "mode",
    `${c.xbox_mode ? "Xbox 360" : t("overlay.native")} · ${c.bluetooth ? "Bluetooth" : "USB"}`,
  );

  const info = element("div", "info");
  info.append(head, mode);
  if (c.percent !== null) info.appendChild(createGauge(c.percent, c.charge));

  item.append(thumb, info);
  return item;
}

function render(payload: OverlayPayload) {
  messageEl.hidden = !payload.message;
  messageEl.textContent = payload.message ?? "";
  listEl.replaceChildren(...payload.controllers.map(renderItem));

  // Restart the entrance animation on every trigger.
  card.style.animation = "none";
  void card.offsetWidth;
  card.style.animation = "";
}

applyTranslations();
listen<OverlayPayload>("overlay", ({ payload }) => render(payload));

// The overlay window lives as long as the app: when the language changes, it reloads to follow it.
listen("language-changed", () => window.location.reload());
