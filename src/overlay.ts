import { listen } from "@tauri-apps/api/event";
import { createGauge, isLow, rangeText, type Charge } from "./battery";
import { createControllerArt } from "./controller-art";
import { applyTranslations, t } from "./i18n";
import { brandIcon, icon, isIcon, type BrandName, type IconName } from "./icons";

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
  pad_color: string | null;
}

// Mirror of menu::MenuView (src-tauri/src/menu.rs).
interface MenuView {
  rows: { icon: string; label: string; value: string | null }[];
  cursor: number;
}

interface OverlayPayload {
  message: string | null;
  icon: string | null;
  menu: MenuView | null;
  controllers: OverlayController[];
}

const card = document.getElementById("card")!;
const messageEl = document.getElementById("message")!;
const listEl = document.getElementById("list")!;
const menuEl = document.getElementById("menu")!;

/** Icon names that the backend sends and that are drawn with another icon (the colour tells the rest). */
const ALIASES: Record<string, IconName> = { "battery-low": "battery" };

/**
 * The modes are shown with the logos of what they present to the games: Xbox for the virtual Xbox 360
 * controller, PlayStation for the native DualSense.
 */
const BRANDS: Record<string, BrandName> = { xbox: "xbox", gamepad: "playstation" };

/** An icon, by name. A name that is not known gives the generic message icon, never an empty tile. */
const glyph = (name: string | null, size = 18): string => {
  if (!name) return "";
  const brand = BRANDS[name];
  const logo = brand ? brandIcon(brand, size) : null;
  if (logo) return logo;
  const resolved = ALIASES[name] ?? name;
  return icon(isIcon(resolved) ? resolved : "message", size);
};

/** The battery level, with an icon when the controller is charging or charged. */
function levelHtml(c: OverlayController): string {
  if (c.charge === "error") return glyph("alert", 16) + t("overlay.error");
  if (c.percent === null) return "—";
  const mark = c.charge === "charging" ? glyph("bolt", 15) : c.charge === "full" ? glyph("check", 15) : "";
  return rangeText(c.percent) + mark;
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
  art.setColorway(c.pad_color);
  const thumb = element("div", "thumb");
  thumb.appendChild(art.el);

  const head = element("div", "head");
  const low = c.percent !== null && isLow(c.percent, c.charge);
  const level = element("span", "level" + (low ? " low" : ""));
  level.innerHTML = levelHtml(c);
  head.append(element("span", "name", c.name), level);

  // Connection and mode as icons: Bluetooth or USB, native (gamepad) or Xbox 360.
  const chips = element("div", "chips");
  const chip = (name: string, title: string) => {
    const el = element("span", "chip");
    el.innerHTML = glyph(name, 16);
    el.title = title;
    chips.append(el);
  };
  chip(c.bluetooth ? "bluetooth" : "usb", c.bluetooth ? "Bluetooth" : "USB");
  chip(c.xbox_mode ? "xbox" : "gamepad", c.xbox_mode ? "Xbox 360" : t("overlay.native"));

  const info = element("div", "info");
  info.append(head, chips);
  if (c.percent !== null) info.appendChild(createGauge(c.percent, c.charge));

  item.append(thumb, info);
  return item;
}

/**
 * The controller menu: one line per row, with its icon, the selected one highlighted. A row with a
 * value shows it between two arrows.
 */
function renderMenu(menu: MenuView | null) {
  menuEl.hidden = !menu;
  if (!menu) return;
  const rows = menu.rows.map((row, index) => {
    const line = element("div", "menu-row" + (index === menu.cursor ? " selected" : ""));
    line.title = row.label;
    const tile = element("span", "tile");
    tile.innerHTML = glyph(row.icon, 20);
    line.append(tile);
    if (row.value === null) {
      line.append(element("span", "menu-value", row.label));
    } else {
      const left = element("span", "arrow");
      left.innerHTML = glyph("chevron-left", 18);
      const right = element("span", "arrow");
      right.innerHTML = glyph("chevron-right", 18);
      line.append(left, element("span", "menu-value", row.value), right);
    }
    return line;
  });
  menuEl.replaceChildren(...rows);
}

/** A short message with its icon. */
function renderMessage(message: string | null, iconName: string | null) {
  messageEl.hidden = !message;
  if (!message) return;
  const tile = element("span", "tile" + (iconName === "alert" || iconName === "battery-low" ? " bad" : ""));
  tile.innerHTML = glyph(iconName ?? "message", 22);
  messageEl.replaceChildren(tile, element("span", "message-text", message));
}

/**
 * The window is hidden, not destroyed: what it showed last would be visible for an instant when it is
 * shown again, before the new content arrives. The card is therefore hidden when the window is, and
 * only shown again once the new content has been drawn.
 */
function clear() {
  card.style.visibility = "hidden";
  card.dataset.menu = "";
}

function render(payload: OverlayPayload) {
  renderMessage(payload.message, payload.icon);
  renderMenu(payload.menu);
  listEl.replaceChildren(...payload.controllers.map(renderItem));

  // Restart the entrance animation when something new appears (not on every move in the menu).
  if (!payload.menu || !card.dataset.menu) {
    card.style.animation = "none";
    void card.offsetWidth;
    card.style.animation = "";
  }
  card.dataset.menu = payload.menu ? "open" : "";
  card.style.visibility = "";
}

applyTranslations();
listen<OverlayPayload>("overlay", ({ payload }) => render(payload));
listen("overlay-clear", clear);

// The overlay window lives as long as the app: when the language changes, it reloads to follow it.
listen("language-changed", () => window.location.reload());
