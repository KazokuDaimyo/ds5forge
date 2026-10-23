// Rule editor: "when this input makes this gesture, then do this" (see src-tauri/src/bindings.rs).
//
// Two lists: the rules (buttons, combinations, stick directions → steps) and the analog bindings
// (a stick drives the mouse, the wheel or keys). Every change is sent whole to the backend, which
// validates it and releases everything the old rules were holding.

import { invoke } from "@tauri-apps/api/core";
import "./rules.css";
import { percent, t, type Key } from "./i18n";
import { icon } from "./icons";
import {
  BUTTON_IDS,
  BUTTON_NAMES,
  type AnalogBinding,
  type AnalogMode,
  type AppAction,
  type KeyInfo,
  type LayerMode,
  type MouseBtn,
  type Output,
  type Profile,
  type Repeat,
  type Rule,
  type RuleGesture,
  type Step,
  type XTarget,
} from "./types";
import { $, hexToRgb, makeSelect, makeSlider, makeSwitch, rgbToHex, type Ctx } from "./ui-common";

// --- Vocabulary ----------------------------------------------------------------------------------

/** Labels from the dictionary for a family of identifiers: `labels("gesture", ["press", …])`. */
const labels = <T extends string>(prefix: string, ids: T[]): [T, string][] =>
  ids.map((id) => [id, t(`${prefix}.${id}` as Key)]);

const STICK_DIRECTIONS: [string, string][] = [
  ["left_stick_up", t("input.stickLeft", { arrow: "↑" })], ["left_stick_right", t("input.stickLeft", { arrow: "→" })],
  ["left_stick_down", t("input.stickLeft", { arrow: "↓" })], ["left_stick_left", t("input.stickLeft", { arrow: "←" })],
  ["right_stick_up", t("input.stickRight", { arrow: "↑" })], ["right_stick_right", t("input.stickRight", { arrow: "→" })],
  ["right_stick_down", t("input.stickRight", { arrow: "↓" })], ["right_stick_left", t("input.stickRight", { arrow: "←" })],
];

const INPUTS: [string, string][] = [
  ...BUTTON_IDS.map((id, i) => [id, BUTTON_NAMES[i]] as [string, string]),
  ...STICK_DIRECTIONS,
];
const inputName = (id: string) => INPUTS.find(([i]) => i === id)?.[1] ?? id;

const GESTURES = labels<RuleGesture>("gesture", ["press", "release", "tap", "hold", "double_tap", "while"]);
const gestureHint = (g: RuleGesture) => t(`gestureHint.${g}` as Key);
const DEFAULT_MS: Partial<Record<RuleGesture, number>> = { tap: 800, hold: 800, double_tap: 300 };
const gestureLabel = (g: RuleGesture) => GESTURES.find(([id]) => id === g)?.[1] ?? g;

const XTARGETS: [XTarget, string][] = [
  ["a", "A"], ["b", "B"], ["x", "X"], ["y", "Y"], ["lb", "LB"], ["rb", "RB"],
  ["back", "Back"], ["start", "Start"], ["lthumb", "L3"], ["rthumb", "R3"], ["guide", "Guide"],
  ["dpad_up", t("xtarget.dpad_up")], ["dpad_down", t("xtarget.dpad_down")],
  ["dpad_left", t("xtarget.dpad_left")], ["dpad_right", t("xtarget.dpad_right")],
  ["lt", t("xtarget.lt")], ["rt", t("xtarget.rt")],
];
const MOUSE_BUTTONS = labels<MouseBtn>("mouse", ["left", "right", "middle", "back", "forward"]);
const OUTPUT_KINDS = labels<Output["kind"]>("output", ["key", "mouse", "pad"]);

const STEP_TYPES = labels<Step["type"]>("step", ["tap", "hold", "down", "up", "wait", "wheel", "mouse_move", "app"]);
const ACTION_TYPES = labels<AppAction["type"]>("action", [
  "toggle_virtual_pad", "show_overlay", "open_menu", "message", "layer", "lightbar", "rumble", "toggle_motion", "power_off",
]);
const LAYER_MODES = labels<LayerMode>("layerMode", ["hold", "toggle", "on", "off"]);

let KEYS: KeyInfo[] = [];
const keyLabel = (id: string) => KEYS.find((k) => k.id === id)?.label ?? id;

// --- Default values --------------------------------------------------------------------------------

const defaultOutput = (kind: Output["kind"]): Output =>
  kind === "pad" ? { kind, target: "a" } : kind === "mouse" ? { kind, button: "left" } : { kind, key: "E" };

function defaultAction(type: AppAction["type"]): AppAction {
  switch (type) {
    case "message": return { type, text: t("default.message") };
    case "lightbar": return { type, r: 255, g: 80, b: 80 };
    case "rumble": return { type, strength: 180, ms: 200 };
    case "layer": return { type, name: t("default.layer"), mode: "hold" };
    default: return { type } as AppAction;
  }
}

function defaultStep(type: Step["type"]): Step {
  switch (type) {
    case "tap": case "hold": case "down": case "up": return { type, output: defaultOutput("key") };
    case "wait": return { type, ms: 100 };
    case "wheel": return { type, amount: -120 };
    case "mouse_move": return { type, dx: 40, dy: 0 };
    case "app": return { type, action: defaultAction("toggle_virtual_pad") };
  }
}

const newRule = (patch: Partial<Rule> = {}): Rule => ({
  enabled: true, layer: null, inputs: ["cross"], gesture: "press", ms: null, steps: [],
  repeat: { type: "none" }, toggle: false, passthrough: false, ...patch,
});

/** The templates offered to start a rule. */
const TEMPLATES: [string, string, () => Rule][] = [
  ["empty", t("template.empty"), () => newRule()],
  ["key", t("template.key"), () => newRule({ gesture: "while", steps: [{ type: "hold", output: { kind: "key", key: "E" } }] })],
  ["turbo", t("template.turbo"), () => newRule({
    gesture: "while", steps: [{ type: "tap", output: { kind: "pad", target: "a" } }], repeat: { type: "until_released", interval_ms: 100 },
  })],
  ["layer", t("template.layer"), () => newRule({
    inputs: ["l1"], gesture: "while", steps: [{ type: "app", action: { type: "layer", name: t("default.layer"), mode: "hold" } }],
  })],
  ["macro", t("template.macro"), () => newRule({
    steps: [{ type: "tap", output: { kind: "key", key: "A" } }, { type: "wait", ms: 100 }, { type: "tap", output: { kind: "key", key: "B" } }],
  })],
  ["lock", t("template.lock"), () => newRule({
    steps: [{ type: "hold", output: { kind: "pad", target: "a" } }], toggle: true,
  })],
  ["longPress", t("template.longPress"), () => newRule({
    gesture: "hold", steps: [{ type: "app", action: { type: "message", text: t("template.longPressMessage") } }],
  })],
];

// --- Descriptions (title of a collapsed rule) --------------------------------------------------------

function describeOutput(o: Output): string {
  switch (o.kind) {
    case "key": return t("desc.key", { key: keyLabel(o.key) });
    case "mouse": return MOUSE_BUTTONS.find(([id]) => id === o.button)?.[1].toLowerCase() ?? o.button;
    case "pad": return t("desc.pad", { target: XTARGETS.find(([id]) => id === o.target)?.[1] ?? o.target });
  }
}

function describeAction(a: AppAction): string {
  switch (a.type) {
    case "message": return t("desc.message", { text: a.text });
    case "layer": {
      const mode = LAYER_MODES.find(([m]) => m === a.mode)?.[1].toLowerCase() ?? a.mode;
      return t("desc.layer", { name: a.name, mode });
    }
    case "lightbar": return t("desc.lightbar");
    case "rumble": return t("desc.rumble");
    default: return ACTION_TYPES.find(([type]) => type === a.type)?.[1].toLowerCase() ?? a.type;
  }
}

function describeStep(s: Step): string {
  switch (s.type) {
    case "tap": return t("desc.tap", { output: describeOutput(s.output) });
    case "hold": return t("desc.hold", { output: describeOutput(s.output) });
    case "down": return t("desc.down", { output: describeOutput(s.output) });
    case "up": return t("desc.up", { output: describeOutput(s.output) });
    case "wait": return t("desc.wait", { ms: s.ms });
    case "wheel": return s.amount < 0 ? t("desc.wheelDown") : t("desc.wheelUp");
    case "mouse_move": return t("desc.mouseMove");
    case "app": return describeAction(s.action);
  }
}

function describeRule(r: Rule): string {
  const inputs = r.inputs.length ? r.inputs.map(inputName).join(" + ") : t("desc.noInput");
  const steps = r.steps.length ? r.steps.map(describeStep).join(t("desc.then")) : t("desc.noStep");
  const extra = [r.toggle ? t("desc.latched") : "", r.repeat.type !== "none" ? t("desc.repeating") : ""]
    .filter(Boolean)
    .join(", ");
  return `${inputs} · ${gestureLabel(r.gesture).toLowerCase()} → ${steps}${extra ? ` (${extra})` : ""}`;
}

// --- Small components -------------------------------------------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

function iconButton(glyph: string, title: string, onClick: () => void, extra = ""): HTMLButtonElement {
  const b = el("button", `icon-btn ${extra}`.trim());
  b.type = "button";
  b.title = title;
  b.setAttribute("aria-label", title);
  b.innerHTML = glyph;
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    onClick();
  });
  return b;
}

function numberInput(value: number, min: number, max: number, unit: string, onChange: (v: number) => void, placeholder = ""): HTMLElement {
  const wrap = el("label", "num");
  const input = el("input");
  input.type = "number";
  input.min = String(min);
  input.max = String(max);
  input.value = Number.isFinite(value) ? String(value) : "";
  input.placeholder = placeholder;
  input.addEventListener("change", () => {
    const raw = input.value.trim();
    if (raw === "") return onChange(NaN);
    const v = Math.min(max, Math.max(min, Math.round(Number(raw))));
    input.value = String(v);
    onChange(v);
  });
  wrap.append(input, unit);
  return wrap;
}

function textInput(value: string, placeholder: string, onChange: (v: string) => void, list?: string): HTMLInputElement {
  const input = el("input", "text");
  input.type = "text";
  input.value = value;
  input.placeholder = placeholder;
  input.maxLength = 40;
  if (list) input.setAttribute("list", list);
  input.addEventListener("change", () => onChange(input.value.trim()));
  return input;
}

function field(label: string, control: HTMLElement): HTMLElement {
  const wrap = el("div", "field");
  wrap.append(el("span", "field-label", label), control);
  return wrap;
}

const keyOptions = (): [string, string][] => KEYS.map((k) => [k.id, k.label]);

// --- Module ---------------------------------------------------------------------------------------------------

export function initRulesUi(ctx: Ctx) {
  const listEl = $("rules-list");
  const analogEl = $("analog-list");
  const layersEl = $("rule-layers") as HTMLDataListElement;
  const templateEl = $("rule-template") as HTMLSelectElement;

  let rules: Rule[] = [];
  let analog: AnalogBinding[] = [];
  /** Expanded rules: remembered by identity, to survive re-renders. */
  const open = new WeakSet<Rule>();

  const saveRules = () => ctx.call("set_rules", { rules });
  const saveAnalog = () => ctx.call("set_analog", { analog });

  function refreshLayers() {
    const names = new Set<string>();
    for (const r of rules) if (r.layer) names.add(r.layer);
    for (const a of analog) if (a.layer) names.add(a.layer);
    for (const r of rules) {
      for (const s of r.steps) if (s.type === "app" && s.action.type === "layer" && s.action.name) names.add(s.action.name);
    }
    layersEl.replaceChildren(...[...names].sort().map((n) => new Option(n)));
  }

  // --- Outputs and actions -------------------------------------------------------------------------------

  /** Choice of an output: the kind (keyboard, mouse, controller) then the key or the button. */
  function outputEditor(get: () => Output, set: (o: Output) => void, changed: () => void): HTMLElement {
    const wrap = el("div", "output");
    const output = get();
    wrap.append(
      makeSelect(OUTPUT_KINDS, output.kind, (kind) => set(defaultOutput(kind))),
    );
    switch (output.kind) {
      case "key":
        wrap.append(makeSelect(keyOptions(), output.key, (key) => { output.key = key; changed(); }));
        break;
      case "mouse":
        wrap.append(makeSelect(MOUSE_BUTTONS, output.button, (button) => { output.button = button; changed(); }));
        break;
      case "pad":
        wrap.append(makeSelect(XTARGETS, output.target, (target) => { output.target = target; changed(); }));
        break;
    }
    return wrap;
  }

  function actionEditor(get: () => AppAction, set: (a: AppAction) => void, changed: () => void): HTMLElement {
    const wrap = el("div", "output");
    const action = get();
    wrap.append(makeSelect(ACTION_TYPES, action.type, (type) => set(defaultAction(type))));
    switch (action.type) {
      case "message":
        wrap.append(textInput(action.text, t("editor.messagePlaceholder"), (v) => { action.text = v; changed(); }));
        break;
      case "layer":
        wrap.append(
          textInput(action.name, t("editor.layerNamePlaceholder"), (v) => { action.name = v; refreshLayers(); changed(); }, "rule-layers"),
          makeSelect(LAYER_MODES, action.mode, (mode) => { action.mode = mode; changed(); }),
        );
        break;
      case "lightbar": {
        const color = el("input", "color");
        color.type = "color";
        color.value = rgbToHex([action.r, action.g, action.b]);
        color.addEventListener("input", () => {
          [action.r, action.g, action.b] = hexToRgb(color.value);
          changed();
        });
        wrap.append(color);
        break;
      }
      case "rumble":
        wrap.append(
          makeSlider(t("editor.strength"), 1, 255, action.strength, "", (v) => { action.strength = v; changed(); }),
          numberInput(action.ms, 20, 5000, "ms", (v) => { if (!Number.isNaN(v)) { action.ms = v; changed(); } }),
        );
        break;
      default:
        break;
    }
    return wrap;
  }

  // --- One rule -------------------------------------------------------------------------------------------------

  function renderRule(rule: Rule, index: number): HTMLElement {
    const card = el("article", "rule");
    card.classList.toggle("off", !rule.enabled);
    card.classList.toggle("open", open.has(rule));

    const title = el("span", "rule-title");
    const updateTitle = () => {
      title.textContent = describeRule(rule);
      card.classList.toggle("incomplete", rule.inputs.length === 0 || rule.steps.length === 0);
    };
    updateTitle();

    // --- header ---
    const header = el("div", "rule-head");
    header.tabIndex = 0;
    header.setAttribute("role", "button");
    const toggleOpen = () => {
      if (open.has(rule)) open.delete(rule); else open.add(rule);
      card.classList.toggle("open", open.has(rule));
    };
    header.addEventListener("click", toggleOpen);
    header.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        toggleOpen();
      }
    });

    const enable = makeSwitch("", rule.enabled, (v) => {
      rule.enabled = v;
      card.classList.toggle("off", !v);
      saveRules();
    });
    enable.classList.add("rule-switch");
    enable.title = t("editor.enable");
    enable.addEventListener("click", (e) => e.stopPropagation());

    const chevron = el("span", "chevron");
    chevron.textContent = "›";

    const layerBadge = el("span", "badge");
    layerBadge.textContent = rule.layer ?? "";
    layerBadge.hidden = !rule.layer;

    const tools = el("span", "rule-tools");
    tools.append(
      iconButton("↑", t("editor.moveUp"), () => move(index, -1)),
      iconButton("↓", t("editor.moveDown"), () => move(index, 1)),
      iconButton("⧉", t("editor.duplicate"), () => {
        const copy: Rule = JSON.parse(JSON.stringify(rule));
        open.add(copy);
        rules.splice(index + 1, 0, copy);
        saveRules();
        renderRules();
      }),
      iconButton(icon("close", 15), t("editor.deleteRule"), () => {
        rules.splice(index, 1);
        saveRules();
        renderRules();
      }, "danger"),
    );
    header.append(enable, chevron, title, layerBadge, tools);

    // --- body ---
    const body = el("div", "rule-body");

    // When
    const when = el("section", "rule-part");
    when.append(el("h4", "", t("editor.when")));

    const chips = el("div", "chips");
    const renderChips = () => {
      chips.replaceChildren(
        ...rule.inputs.map((input) => {
          const chip = el("span", "chip");
          chip.append(inputName(input));
          chip.append(iconButton(icon("close", 12), t("editor.remove", { name: inputName(input) }), () => {
            rule.inputs = rule.inputs.filter((i) => i !== input);
            saveRules();
            renderChips();
            updateTitle();
          }, "chip-x"));
          return chip;
        }),
      );
      if (rule.inputs.length < 4) {
        const add = makeSelect<string>(
          [["", rule.inputs.length ? t("editor.addToCombo") : t("editor.chooseInput")], ...INPUTS],
          "",
          (value) => {
            if (!value || rule.inputs.includes(value)) return;
            rule.inputs.push(value);
            saveRules();
            renderChips();
            updateTitle();
          },
        );
        add.classList.add("chip-add");
        chips.append(add);
      }
    };
    renderChips();

    const gestureHintEl = el("p", "hint", gestureHint(rule.gesture));
    const durationSlot = el("span", "duration");
    const renderDuration = () => {
      durationSlot.replaceChildren();
      const fallback = DEFAULT_MS[rule.gesture];
      if (fallback === undefined) return;
      durationSlot.append(
        numberInput(rule.ms ?? NaN, 50, 10000, "ms", (v) => { rule.ms = Number.isNaN(v) ? null : v; saveRules(); }, String(fallback)),
      );
    };
    renderDuration();

    const gestureSelect = makeSelect(GESTURES, rule.gesture, (gesture) => {
      rule.gesture = gesture;
      gestureHintEl.textContent = gestureHint(gesture);
      renderDuration();
      renderOptions();
      saveRules();
      updateTitle();
    });

    const layerInput = textInput(rule.layer ?? "", t("editor.layerPlaceholder"), (v) => {
      rule.layer = v === "" ? null : v;
      layerBadge.textContent = rule.layer ?? "";
      layerBadge.hidden = !rule.layer;
      refreshLayers();
      saveRules();
    }, "rule-layers");

    const whenGrid = el("div", "fields");
    whenGrid.append(
      field(t("editor.inputs"), chips),
      field(t("editor.gesture"), (() => { const w = el("div", "output"); w.append(gestureSelect, durationSlot); return w; })()),
      field(t("editor.layer"), layerInput),
    );
    when.append(whenGrid, gestureHintEl);

    // Then
    const then = el("section", "rule-part");
    then.append(el("h4", "", t("editor.then")));
    const stepsEl = el("div", "steps");

    function renderSteps() {
      stepsEl.replaceChildren();
      if (rule.steps.length === 0) stepsEl.append(el("p", "hint", t("editor.noSteps")));
      rule.steps.forEach((step, i) => {
        const row = el("div", "step");
        row.append(el("span", "step-n", String(i + 1)));

        const type = makeSelect(STEP_TYPES, step.type, (stepType) => {
          rule.steps[i] = defaultStep(stepType);
          saveRules();
          renderSteps();
          updateTitle();
        });

        const params = el("div", "step-params");
        const changed = () => { saveRules(); updateTitle(); };
        switch (step.type) {
          case "tap": case "hold": case "down": case "up":
            params.append(outputEditor(() => step.output, (o) => { step.output = o; changed(); renderSteps(); }, changed));
            break;
          case "wait":
            params.append(numberInput(step.ms, 0, 60000, "ms", (v) => { if (!Number.isNaN(v)) { step.ms = v; saveRules(); updateTitle(); } }));
            break;
          case "wheel":
            params.append(makeSelect<string>(
              [["120", t("wheel.up1")], ["-120", t("wheel.down1")], ["360", t("wheel.up3")], ["-360", t("wheel.down3")]],
              String(step.amount),
              (v) => { step.amount = Number(v); saveRules(); updateTitle(); },
            ));
            break;
          case "mouse_move":
            params.append(
              field(t("editor.horizontal"), numberInput(step.dx, -2000, 2000, "px", (v) => { if (!Number.isNaN(v)) { step.dx = v; saveRules(); } })),
              field(t("editor.vertical"), numberInput(step.dy, -2000, 2000, "px", (v) => { if (!Number.isNaN(v)) { step.dy = v; saveRules(); } })),
            );
            break;
          case "app":
            params.append(actionEditor(() => step.action, (a) => { step.action = a; changed(); renderSteps(); refreshLayers(); }, changed));
            break;
        }

        const tools = el("span", "rule-tools");
        tools.append(
          iconButton("↑", t("editor.moveStepUp"), () => moveStep(i, -1)),
          iconButton("↓", t("editor.moveStepDown"), () => moveStep(i, 1)),
          iconButton(icon("close", 15), t("editor.deleteStep"), () => {
            rule.steps.splice(i, 1);
            saveRules();
            renderSteps();
            updateTitle();
          }, "danger"),
        );
        row.append(type, params, tools);
        stepsEl.append(row);
      });
    }

    function moveStep(i: number, delta: number) {
      const j = i + delta;
      if (j < 0 || j >= rule.steps.length) return;
      [rule.steps[i], rule.steps[j]] = [rule.steps[j], rule.steps[i]];
      saveRules();
      renderSteps();
      updateTitle();
    }

    const addStep = makeSelect<string>(
      [["", t("editor.addStep")], ...STEP_TYPES],
      "",
      (value) => {
        if (!value) return;
        rule.steps.push(defaultStep(value as Step["type"]));
        saveRules();
        renderSteps();
        refreshLayers();
        updateTitle();
      },
    );
    addStep.classList.add("add-step");
    then.append(stepsEl, addStep);
    renderSteps();

    // Options
    const options = el("section", "rule-part");
    options.append(el("h4", "", t("editor.options")));
    const optionsEl = el("div", "options");
    function renderOptions() {
      optionsEl.replaceChildren();
      const repeatKind = rule.repeat.type;
      const repeatSelect = makeSelect<Repeat["type"]>(
        [
          ["none", t("editor.repeatNone")],
          ["until_released", t("editor.repeatUntilReleased")],
          ["times", t("editor.repeatTimes")],
        ],
        repeatKind,
        (type) => {
          rule.repeat = type === "none" ? { type } : type === "times"
            ? { type, count: 3, interval_ms: 100 }
            : { type, interval_ms: 100 };
          saveRules();
          updateTitle();
          renderOptions();
        },
      );
      const repeatRow = el("div", "output");
      repeatRow.append(repeatSelect);
      if (rule.repeat.type !== "none") {
        const repeat = rule.repeat;
        if (repeat.type === "times") {
          repeatRow.append(numberInput(repeat.count, 1, 1000, t("editor.times"), (v) => { if (!Number.isNaN(v)) { repeat.count = v; saveRules(); } }));
        }
        repeatRow.append(numberInput(repeat.interval_ms, 10, 60000, t("editor.msBetween"), (v) => { if (!Number.isNaN(v)) { repeat.interval_ms = v; saveRules(); } }));
      }
      optionsEl.append(field(t("editor.repeat"), repeatRow));
      if (rule.repeat.type === "until_released" && rule.gesture !== "while") {
        optionsEl.append(el("p", "hint error", t("editor.repeatNeedsWhile")));
      }
      optionsEl.append(
        makeSwitch(t("editor.latch"), rule.toggle, (v) => {
          rule.toggle = v;
          saveRules();
          updateTitle();
        }),
        makeSwitch(t("editor.passthrough"), rule.passthrough, (v) => {
          rule.passthrough = v;
          saveRules();
        }),
      );
    }
    renderOptions();
    options.append(optionsEl);

    body.append(when, then, options);
    card.append(header, body);
    return card;
  }

  function move(index: number, delta: number) {
    const j = index + delta;
    if (j < 0 || j >= rules.length) return;
    [rules[index], rules[j]] = [rules[j], rules[index]];
    saveRules();
    renderRules();
  }

  function renderRules() {
    refreshLayers();
    listEl.replaceChildren();
    if (rules.length === 0) {
      listEl.append(el("p", "empty-rules", t("rules.empty")));
      return;
    }
    rules.forEach((rule, i) => listEl.append(renderRule(rule, i)));
  }

  // --- Analog bindings ------------------------------------------------------------------------------------------

  const ANALOG_MODES = labels<AnalogMode["type"]>("analogMode", ["mouse", "scroll", "keys"]);

  function renderAnalog() {
    refreshLayers();
    analogEl.replaceChildren();
    if (analog.length === 0) {
      analogEl.append(el("p", "empty-rules", t("rules.emptyAnalog")));
      return;
    }
    analog.forEach((binding, i) => {
      const card = el("article", "analog");
      card.classList.toggle("off", !binding.enabled);

      const top = el("div", "analog-top");
      const enable = makeSwitch("", binding.enabled, (v) => {
        binding.enabled = v;
        card.classList.toggle("off", !v);
        saveAnalog();
      });
      enable.classList.add("rule-switch");

      const stick = makeSelect<string>(
        [["left", t("controls.leftStick")], ["right", t("controls.rightStick")]],
        binding.stick,
        (v) => {
          binding.stick = v as AnalogBinding["stick"];
          saveAnalog();
        },
      );
      const mode = makeSelect(ANALOG_MODES, binding.mode.type, (type) => {
        binding.mode =
          type === "mouse" ? { type, speed: 40 }
            : type === "scroll" ? { type, speed: 40 }
              : { type, up: "W", down: "S", left: "A", right: "D" };
        saveAnalog();
        renderAnalog();
      });
      const remove = iconButton(icon("close", 15), t("editor.removeBinding"), () => {
        analog.splice(i, 1);
        saveAnalog();
        renderAnalog();
      }, "danger");
      top.append(enable, stick, mode, remove);

      const details = el("div", "analog-details");
      const m = binding.mode;
      if (m.type === "mouse" || m.type === "scroll") {
        details.append(makeSlider(t("editor.speed"), 1, 100, m.speed, "", (v) => { m.speed = v; saveAnalog(); }));
      } else {
        const keysGrid = el("div", "keys-grid");
        for (const dir of ["up", "left", "down", "right"] as const) {
          keysGrid.append(field(t(`editor.${dir}`), makeSelect(keyOptions(), m[dir], (key) => { m[dir] = key; saveAnalog(); })));
        }
        details.append(keysGrid);
      }
      details.append(
        makeSlider(t("controls.deadzone"), 0, 60, binding.deadzone, percent, (v) => { binding.deadzone = v; saveAnalog(); }),
        field(t("editor.layer"), textInput(binding.layer ?? "", t("editor.layerPlaceholder"), (v) => {
          binding.layer = v === "" ? null : v;
          refreshLayers();
          saveAnalog();
        }, "rule-layers")),
        makeSwitch(t("editor.consume"), binding.consume, (v) => { binding.consume = v; saveAnalog(); }),
      );

      card.append(top, details);
      analogEl.append(card);
    });
  }

  // --- Add buttons -----------------------------------------------------------------------------------------------

  templateEl.replaceChildren(
    new Option(t("rules.addTemplate"), ""),
    ...TEMPLATES.map(([id, label]) => new Option(label, id)),
  );
  templateEl.addEventListener("change", () => {
    const template = TEMPLATES.find(([id]) => id === templateEl.value);
    templateEl.value = "";
    if (!template || rules.length >= 64) return;
    const rule = template[2]();
    open.add(rule);
    rules.push(rule);
    saveRules();
    renderRules();
    listEl.lastElementChild?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  });

  $("analog-add").addEventListener("click", () => {
    if (analog.length >= 8) return;
    analog.push({ enabled: true, layer: null, stick: "right", mode: { type: "mouse", speed: 40 }, deadzone: 15, consume: true });
    saveAnalog();
    renderAnalog();
  });

  $("rules-reset").addEventListener("click", () => {
    if (rules.length && !confirm(t("rules.confirmDeleteAll"))) return;
    rules.length = 0;
    saveRules();
    renderRules();
  });

  // The list of keys comes from the backend (keys.rs): a single source for names and codes.
  invoke<KeyInfo[]>("list_keys")
    .then((keys) => {
      KEYS = keys;
      renderRules();
      renderAnalog();
    })
    .catch(() => {});

  return {
    load(profile: Profile) {
      rules = profile.rules ??= [];
      analog = profile.analog ??= [];
      renderRules();
      renderAnalog();
    },
  };
}
