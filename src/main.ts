import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initAppUi } from "./app-ui";
import { createControllerArt, type ArtState } from "./controller-art";
import { chargeMark, createGauge, rangeText, type Charge } from "./battery";
import { initComponentsUi } from "./components-ui";
import { initDiagnosticsUi } from "./diagnostics-ui";
import { applyTranslations, lang, storeLanguage, t, type Key, type Lang } from "./i18n";
import { hydrateIcons } from "./icons";
import { initInputUi } from "./input-ui";
import { initLightingUi } from "./lighting-ui";
import { initMotionUi } from "./motion-ui";
import { initRulesUi } from "./rules-ui";
import {
  type ControllerInfo,
  type Kind,
  type Profile,
  type Side,
  type StateEvent,
  type TriggerEffect,
} from "./types";
import { $, type Ctx } from "./ui-common";
import { initTitlebar } from "./titlebar";
import { initViews, onViewChange, refreshViews, showControllerView } from "./views";

// The static markup is filled in first: some elements below read their default text.
applyTranslations();

// --- State ---------------------------------------------------------------------------

let controllers: ControllerInfo[] = [];
let selected: string | null = null;

/** Last known battery of each controller, fed by the state events. */
const batteries = new Map<string, { percent: number | null; charge: Charge }>();

function batteryText(id: string): string {
  const b = batteries.get(id);
  if (!b || b.percent === null) return "";
  return rangeText(b.percent) + chargeMark(b.charge);
}

const current = () => controllers.find((c) => c.id === selected) ?? null;

/** Applies a setting to the local copy of the profile (the backend has its own copy). */
const editLocal = (fn: (p: Profile) => void) => {
  const c = current();
  if (c) fn(c.profile);
};

/** Calls a backend command for the selected controller. */
const call = (cmd: string, args: Record<string, unknown> = {}) => {
  if (selected) invoke(cmd, { id: selected, ...args });
};

// --- Tabs ----------------------------------------------------------------------------

const tabsEl = $("tabs");

const rgb = ([r, g, b]: number[]) => `rgb(${r}, ${g}, ${b})`;
const hex = ([r, g, b]: number[]) =>
  "#" + [r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("");

function renderTabs() {
  tabsEl.replaceChildren();
  for (const c of controllers) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "side-item" + (c.id === selected ? " active" : "") + (c.connected ? "" : " offline");
    item.style.setProperty("--item-color", accentOf(c.profile.lightbar));

    const dot = document.createElement("span");
    dot.className = "side-dot";

    const name = document.createElement("span");
    name.className = "side-name";
    name.textContent = c.profile.name || c.id;

    const meta = document.createElement("span");
    meta.className = "side-meta";
    const mode = c.bluetooth ? "Bluetooth" : "USB";
    meta.textContent = c.connected ? [mode, batteryText(c.id)].filter(Boolean).join(" · ") : t("status.offline");

    item.append(dot, name, meta);
    item.addEventListener("click", () => select(c.id));
    tabsEl.appendChild(item);
  }
  refreshViews(controllers.length > 0);
}

/** A switched-off (black) light bar has no tint to show: fall back to the accent colour. */
function accentOf([r, g, b]: number[]): string {
  return r + g + b === 0 ? "var(--accent)" : rgb([r, g, b]);
}

/** The tint of the header and of the drawing follows the light bar colour of the controller on display. */
function paintAccent() {
  const c = current();
  if (c) $("controller-view").style.setProperty("--led", accentOf(c.profile.lightbar));
}

function renderStatus() {
  const c = current();
  const status = $("status");
  status.textContent = !c?.connected ? t("status.offline") : c.bluetooth ? "Bluetooth" : "USB";
  status.className = `status ${c?.connected ? "on" : "off"}`;

  const power = $<HTMLButtonElement>("power-off");
  power.disabled = !(c?.connected && c.bluetooth);
  power.title = !c?.connected
    ? t("power.notConnected")
    : c.bluetooth
      ? ""
      : t("power.usb");
  renderBattery();
}

function renderBattery() {
  const pill = $("battery");
  const text = $("battery-text");
  const gauge = $("battery-gauge");
  const c = current();
  const b = c && c.connected ? batteries.get(c.id) : undefined;

  gauge.replaceChildren();
  if (!b) {
    text.textContent = t("battery.none");
    pill.className = "status battery off";
  } else if (b.charge === "error") {
    text.textContent = t("battery.error");
    pill.className = "status battery bad";
  } else if (b.percent === null) {
    text.textContent = t("battery.none");
    pill.className = "status battery off";
  } else {
    const state = {
      charging: t("battery.charging"),
      full: t("battery.charged"),
      discharging: "",
      unknown: "",
      error: "",
    }[b.charge];
    gauge.appendChild(createGauge(b.percent, b.charge));
    text.textContent = rangeText(b.percent) + state;
    pill.className = "status battery on";
  }
}

async function refresh() {
  controllers = await invoke<ControllerInfo[]>("list_controllers");
  for (const c of controllers) if (!c.connected) batteries.delete(c.id);
  const selectionChanged = !controllers.some((c) => c.id === selected);
  if (selectionChanged) {
    selected = (controllers.find((c) => c.connected) ?? controllers[0])?.id ?? null;
  }
  renderTabs();
  renderStatus();
  if (selectionChanged) loadProfile();
  if (!current()?.connected) resetLive();
}

function select(id: string) {
  // Clicking the controller that is already selected still leaves a global page (settings, diagnostics).
  showControllerView();
  if (id === selected) return;
  stopRumble(); // do not leave the controller we are leaving rumbling
  selected = id;
  renderTabs();
  renderStatus();
  loadProfile();
  resetLive();
}

// --- Live display ----------------------------------------------------------------------

const art = createControllerArt();
$("controller-art").appendChild(art.el);

const showState = (s: ArtState) => art.update(s);
const resetLive = () => showState({ lx: 128, ly: 128, rx: 128, ry: 128, l2: 0, r2: 0, buttons: 0 });

listen<StateEvent>("controller-state", ({ payload }) => {
  // The battery changes slowly: the tabs are only redrawn when it moved.
  const previous = batteries.get(payload.id);
  if (previous?.percent !== payload.battery || previous?.charge !== payload.charge) {
    batteries.set(payload.id, { percent: payload.battery, charge: payload.charge });
    renderTabs();
    renderBattery();
  }
  if (payload.id === selected) showState(payload);
});

// --- Settings of the selected controller ---------------------------------------------------

const nameInput = $<HTMLInputElement>("name");
nameInput.addEventListener("change", () => {
  const name = nameInput.value.trim();
  editLocal((p) => (p.name = name));
  renderTabs();
  call("rename_controller", { name });
});

// Light bar
const colorInput = $<HTMLInputElement>("color");
const hexToRgb = (h: string): [number, number, number] => [
  parseInt(h.slice(1, 3), 16),
  parseInt(h.slice(3, 5), 16),
  parseInt(h.slice(5, 7), 16),
];
const setLightbar = ([r, g, b]: [number, number, number]) => {
  editLocal((p) => (p.lightbar = [r, g, b]));
  call("set_lightbar", { r, g, b });
  renderTabs();
  paintAccent();
};
colorInput.addEventListener("input", () => setLightbar(hexToRgb(colorInput.value)));
$("color-off").addEventListener("click", () => {
  colorInput.value = "#000000";
  setLightbar([0, 0, 0]);
});

// Player LEDs: bit i = LED i (from left to right)
let playerMask = 0;
const ledEls: HTMLButtonElement[] = [];
const showLeds = () =>
  ledEls.forEach((el, i) => el.classList.toggle("active", (playerMask & (1 << i)) !== 0));
for (let i = 0; i < 5; i++) {
  const led = document.createElement("button");
  led.type = "button";
  led.className = "led";
  led.addEventListener("click", () => {
    playerMask ^= 1 << i;
    showLeds();
    editLocal((p) => (p.player_leds = playerMask));
    call("set_player_leds", { mask: playerMask });
  });
  ledEls.push(led);
  $("player-leds").appendChild(led);
}

// Power off (Bluetooth only)
const powerMsg = $("power-msg");
$("power-off").addEventListener("click", async () => {
  const id = selected;
  if (!id) return;
  powerMsg.classList.remove("error");
  try {
    await invoke("power_off_controller", { id });
    powerMsg.textContent = t("power.requested");
    setTimeout(() => (powerMsg.textContent = ""), 3000);
  } catch (error) {
    powerMsg.textContent = String(error);
    powerMsg.classList.add("error");
  }
});

// Rumble (one-off: not saved in the profile)
const rumbleL = $<HTMLInputElement>("rumble-l");
const rumbleR = $<HTMLInputElement>("rumble-r");
const sendRumble = () =>
  call("set_rumble", { left: Number(rumbleL.value), right: Number(rumbleR.value) });
rumbleL.addEventListener("input", sendRumble);
rumbleR.addEventListener("input", sendRumble);

function stopRumble() {
  const active = rumbleL.value !== "0" || rumbleR.value !== "0";
  rumbleL.value = "0";
  rumbleR.value = "0";
  if (active) sendRumble();
}
$("rumble-stop").addEventListener("click", () => {
  rumbleL.value = "0";
  rumbleR.value = "0";
  sendRumble();
});
// Test pulse: both motors at full power for 1 s, regardless of the sliders.
$("rumble-pulse").addEventListener("click", () => {
  const id = selected;
  if (!id) return;
  invoke("set_rumble", { id, left: 255, right: 255 });
  setTimeout(() => invoke("set_rumble", { id, left: 0, right: 0 }), 1000);
});

$("overlay-preview").addEventListener("click", () => call("preview_overlay"));

// Virtual controller
const padToggle = $<HTMLInputElement>("virtual-pad");
const padMsg = $("virtual-pad-msg");
const padHint = padMsg.textContent ?? "";
padToggle.addEventListener("change", () => {
  editLocal((p) => (p.virtual_pad = padToggle.checked));
  call("set_virtual_pad", { enabled: padToggle.checked });
});

// Hiding the physical controller (HidHide) rewrites the card's message.
listen<{ id: string; hidden: boolean; error: string | null }>("cloak", ({ payload }) => {
  if (payload.id !== selected) return;
  padMsg.textContent = payload.error
    ? t("pad.cloakError", { error: payload.error })
    : payload.hidden
      ? t("pad.hidden")
      : padHint;
  padMsg.classList.toggle("error", payload.error !== null);
});

// The backend confirms the real state (or reports an error, for example a missing driver).
listen<{ id: string; enabled: boolean; error: string | null }>("virtual-pad", ({ payload }) => {
  const c = controllers.find((x) => x.id === payload.id);
  if (c) c.profile.virtual_pad = payload.enabled;
  if (payload.id !== selected) return;
  padToggle.checked = payload.enabled;
  padMsg.textContent = payload.error ?? padHint;
  padMsg.classList.toggle("error", payload.error !== null);
});

// --- Adaptive triggers -----------------------------------------------------------------

interface ParamDef {
  key: string;
  label: Key;
  min: number;
  max: number;
  value: number;
}

// The keys match the fields of `TriggerEffect` on the Rust side.
const EFFECTS: Record<Kind, { label: Key; params: ParamDef[] }> = {
  off: { label: "effect.off", params: [] },
  feedback: {
    label: "effect.feedback",
    params: [
      { key: "start", label: "param.start", min: 0, max: 9, value: 3 },
      { key: "strength", label: "param.strength", min: 1, max: 8, value: 6 },
    ],
  },
  weapon: {
    label: "effect.weapon",
    params: [
      { key: "start", label: "param.engage", min: 2, max: 7, value: 3 },
      { key: "end", label: "param.release", min: 3, max: 8, value: 6 },
      { key: "strength", label: "param.strength", min: 1, max: 8, value: 8 },
    ],
  },
  vibration: {
    label: "effect.vibration",
    params: [
      { key: "start", label: "param.start", min: 0, max: 9, value: 2 },
      { key: "amplitude", label: "param.amplitude", min: 1, max: 8, value: 6 },
      { key: "frequency", label: "param.frequency", min: 1, max: 255, value: 30 },
    ],
  },
};

function buildTriggerPanel(side: Side, title: string) {
  const wrap = document.createElement("div");
  wrap.className = "trigger";
  const heading = document.createElement("h3");
  heading.textContent = title;
  wrap.appendChild(heading);

  const select = document.createElement("select");
  for (const [kind, def] of Object.entries(EFFECTS)) select.add(new Option(t(def.label), kind));
  wrap.appendChild(select);

  const paramsEl = document.createElement("div");
  wrap.appendChild(paramsEl);

  const values: Record<string, number> = {};

  const send = () => {
    const kind = select.value as Kind;
    const p = { ...values };
    // For the weapon, the release must come strictly after the engage point.
    if (kind === "weapon" && p.end <= p.start) p.end = p.start + 1;
    const effect = { kind, ...p } as TriggerEffect;
    editLocal((profile) => (profile[`trigger_${side}`] = effect));
    call("set_trigger", { side, effect });
  };

  /** Builds the sliders of the chosen effect, without sending anything to the controller. */
  const renderParams = (initial: Partial<Record<string, number | string>> = {}) => {
    paramsEl.replaceChildren();
    for (const k of Object.keys(values)) delete values[k];
    for (const def of EFFECTS[select.value as Kind].params) {
      const start = Number(initial[def.key] ?? def.value);
      values[def.key] = start;

      const label = document.createElement("label");
      label.className = "slider";
      const out = document.createElement("output");
      out.textContent = String(start);
      const input = document.createElement("input");
      input.type = "range";
      input.min = String(def.min);
      input.max = String(def.max);
      input.value = String(start);
      input.addEventListener("input", () => {
        values[def.key] = Number(input.value);
        out.textContent = input.value;
        send();
      });
      label.append(t(def.label), input, out);
      paramsEl.appendChild(label);
    }
  };

  select.addEventListener("change", () => {
    renderParams();
    send();
  });

  document.getElementById("triggers")!.appendChild(wrap);

  return {
    /** Shows the effect of the profile, without sending it back to the controller. */
    load(effect: TriggerEffect | null) {
      select.value = effect?.kind ?? "off";
      renderParams(effect ?? {});
    },
  };
}

const triggerPanels = {
  left: buildTriggerPanel("left", t("triggers.left")),
  right: buildTriggerPanel("right", t("triggers.right")),
};

// --- Loading the profile into the interface ------------------------------------------------

function loadProfile() {
  const c = current();
  if (!c) return;
  const p = c.profile;

  nameInput.value = p.name;
  $("ctrl-id").textContent = c.id;
  paintAccent();

  colorInput.value = hex(p.lightbar);
  playerMask = p.player_leds;
  showLeds();

  powerMsg.textContent = "";
  padToggle.checked = p.virtual_pad;
  padMsg.textContent = padHint;
  padMsg.classList.remove("error");

  triggerPanels.left.load(p.trigger_left);
  triggerPanels.right.load(p.trigger_right);
  lightingUi.load(p);
  inputUi.load(p);
  rulesUi.load(p);
  motionUi.load(p);

  rumbleL.value = "0";
  rumbleR.value = "0";
}

// --- HidHide ---------------------------------------------------------------------------

const hidhideToggle = $<HTMLInputElement>("hidhide-auto");
const hidhideStatus = $("hidhide-status");

interface CloakStatus {
  installed: boolean;
  cloak_active: boolean | null;
  app_registered: boolean | null;
  error: string | null;
}

function showHidhideStatus(extraError?: string) {
  invoke<CloakStatus>("hidhide_status").then((s) => {
    const problem = extraError ?? s.error;
    if (!s.installed) {
      hidhideStatus.textContent = t("hidhide.notInstalled");
    } else if (problem) {
      hidhideStatus.textContent = problem;
    } else {
      const state = s.cloak_active ? t("hidhide.active") : t("hidhide.inactive");
      hidhideStatus.textContent = t("hidhide.detected", { state });
    }
    hidhideStatus.classList.toggle("error", !s.installed || problem !== null);
  });
}

hidhideToggle.addEventListener("change", () => {
  invoke("set_hidhide_auto", { enabled: hidhideToggle.checked })
    .then(() => showHidhideStatus())
    .catch((error) => showHidhideStatus(String(error)));
});

// The saved language is the reference: the backend keeps it, and the window follows it. On the
// very first start nothing is saved yet, so the language detected here is handed to the backend
// (it translates the tray menu and the overlay messages).
invoke<{ hidhide_auto: boolean; language: Lang | null }>("get_settings").then((s) => {
  hidhideToggle.checked = s.hidhide_auto;
  showHidhideStatus();
  if (s.language && s.language !== lang) {
    storeLanguage(s.language);
    window.location.reload();
  } else if (!s.language) {
    invoke("set_language", { language: lang });
  }
});

// The status text reflects the *current* state of HidHide: it is read again whenever it may have changed
// (otherwise a past error, for example HidHide being busy at start-up, would stay displayed wrongly).
onViewChange((view) => {
  if (view === "settings") showHidhideStatus();
});
listen("cloak", () => showHidhideStatus());
window.addEventListener("focus", () => showHidhideStatus());

// --- UDP server ------------------------------------------------------------------------

const udpStatus = $("udp-status");
const udpLast = $("udp-last");

invoke<{ listening: boolean; port: number; error: string | null }>("get_udp_status").then((s) => {
  udpStatus.textContent = s.listening
    ? t("udp.listening", { port: s.port })
    : (s.error ?? t("udp.stopped"));
  udpStatus.classList.toggle("error", !s.listening);
});

listen<{ text: string; error: string | null }>("udp-packet", ({ payload }) => {
  udpLast.textContent = payload.error ? `${payload.error}\n${payload.text}` : payload.text;
  udpLast.classList.toggle("error", payload.error !== null);
});

// --- Modules ---------------------------------------------------------------------------

const ctx: Ctx = { current, editLocal, call };
const lightingUi = initLightingUi(ctx);
const inputUi = initInputUi(ctx);
const motionUi = initMotionUi(ctx);
const rulesUi = initRulesUi(ctx);
initAppUi();
initComponentsUi();
initDiagnosticsUi();
hydrateIcons();
initTitlebar();
initViews();

// --- Start-up ------------------------------------------------------------------------------

listen("controllers-changed", () => refresh());
refresh();
