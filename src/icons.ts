import { normalizeSvg } from "./brand";

// Interface icons: simple 24×24 strokes, drawn as outlines so that they inherit the text colour.

const PATHS: Record<string, string> = {
  gamepad:
    '<rect x="2" y="7" width="20" height="11" rx="5.5"/><path d="M7 10.5v4M5 12.5h4"/><circle cx="15.5" cy="11.5" r=".9"/><circle cx="18" cy="14" r=".9"/>',
  sliders:
    '<path d="M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1"/><circle cx="15" cy="6" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="17" cy="18" r="2"/>',
  rules: '<path d="M9 6h11M9 12h11M9 18h11"/><path d="m3.5 6 1 1 2-2M3.5 12l1 1 2-2M3.5 18l1 1 2-2"/>',
  bolt: '<path d="M13 2.5 5 13.5h6l-1 8 8-11h-6z"/>',
  bulb: '<path d="M9.5 18.5h5M10.5 21.5h3"/><path d="M12 3a6 6 0 0 0-3.8 10.6c.8.7 1.3 1.6 1.3 2.9h5c0-1.3.5-2.2 1.3-2.9A6 6 0 0 0 12 3z"/>',
  motion: '<path d="M20.5 12a8.5 8.5 0 1 1-2.6-6.1"/><path d="M20.5 3.5v4.4h-4.4"/><circle cx="12" cy="12" r="1.6"/>',
  settings:
    '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
  layers: '<path d="m12 3 9 5-9 5-9-5 9-5z"/><path d="m3 12.5 9 5 9-5"/><path d="m3 17 9 5 9-5"/>',
  battery: '<rect x="2.5" y="8" width="16" height="9" rx="2.5"/><path d="M21 11v3"/>',
  alert: '<path d="M12 3.5 21.5 20h-19z"/><path d="M12 10v4.5M12 17.5v.01"/>',
  xbox: '<circle cx="12" cy="12" r="9"/><path d="m8.5 8.5 7 7M15.5 8.5l-7 7"/>',
  message: '<path d="M4 5.5h16a1 1 0 0 1 1 1V16a1 1 0 0 1-1 1h-8l-4.5 3.5V17H4a1 1 0 0 1-1-1V6.5a1 1 0 0 1 1-1z"/>',
  "chevron-left": '<path d="m14.5 6-6 6 6 6"/>',
  "chevron-right": '<path d="m9.5 6 6 6-6 6"/>',
  check: '<path d="m5 12.5 4.5 4.5L19 7.5"/>',
  pulse: '<path d="M3 12h4l2.5-7 4 14 2.5-7H21"/>',
  power: '<path d="M12 3v8"/><path d="M6.4 6.4a8 8 0 1 0 11.2 0"/>',
  bluetooth: '<path d="m7 7 10 10-5 4.5V2.5L17 7 7 17"/>',
  usb: '<circle cx="12" cy="19" r="2"/><path d="M12 17V4M12 4 9.5 7M12 4l2.5 3M12 12l4-3v-1.5M12 14l-4-2.5"/><rect x="14.5" y="5.5" width="3" height="3"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  close: '<path d="M6 6l12 12M18 6 6 18"/>',
  minus: '<path d="M5 12h14"/>',
  square: '<rect x="5.5" y="5.5" width="13" height="13" rx="2"/>',
  restore:
    '<rect x="4.5" y="8.5" width="11" height="11" rx="2"/><path d="M8.5 8.5V6.5a2 2 0 0 1 2-2h7a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-2"/>',
};

export type IconName = keyof typeof PATHS;

/** The logo files in `src/assets/brand/` (`xbox.svg`, `playstation.svg`; see the README there). */
const BRAND_FILES = import.meta.glob("./assets/brand/*.svg", { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;

export type BrandName = "xbox" | "playstation";

/** A brand logo as an inline icon (`null` only if the file cannot be read as an SVG). */
export function brandIcon(name: BrandName, size = 18): string | null {
  const raw = BRAND_FILES[`./assets/brand/${name}.svg`];
  return raw ? normalizeSvg(raw, size) : null;
}

export const isIcon = (name: string): name is IconName => name in PATHS;

export function icon(name: IconName, size = 18): string {
  return (
    `<svg class="icon" viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" ` +
    `stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${PATHS[name]}</svg>`
  );
}

/** Inserts the icon of every element with `data-icon="name"` at its start. */
export function hydrateIcons(root: ParentNode = document) {
  root.querySelectorAll<HTMLElement>("[data-icon]").forEach((el) => {
    if (el.querySelector(":scope > svg.icon")) return;
    const size = Number(el.dataset.iconSize) || 18;
    el.insertAdjacentHTML("afterbegin", icon(el.dataset.icon as IconName, size));
  });
}
