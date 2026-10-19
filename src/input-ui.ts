// Stick tuning (dead zone, outer limit, curve) and simple button remapping.

import { percent, t } from "./i18n";
import { BUTTON_IDS, BUTTON_NAMES, type Curve, type InputConfig, type Profile, type StickConfig, type XTarget } from "./types";
import { $, makeSelect, makeSlider, type Ctx } from "./ui-common";

const TARGETS: [XTarget, string][] = [
  ["a", "A"], ["b", "B"], ["x", "X"], ["y", "Y"],
  ["lb", "LB"], ["rb", "RB"], ["back", "Back"], ["start", "Start"],
  ["lthumb", t("controls.leftStickClick")], ["rthumb", t("controls.rightStickClick")], ["guide", "Guide"],
  ["dpad_up", t("xtarget.dpad_up")], ["dpad_down", t("xtarget.dpad_down")],
  ["dpad_left", t("xtarget.dpad_left")], ["dpad_right", t("xtarget.dpad_right")],
  ["lt", "LT"], ["rt", "RT"],
  ["none", t("controls.disabled")],
];

/** Default mapping (mirror of `default_target` in input.rs). */
const DEFAULT_TARGET: Record<string, XTarget> = {
  square: "x", cross: "a", circle: "b", triangle: "y",
  l1: "lb", r1: "rb", l2: "lt", r2: "rt",
  share: "back", options: "start", l3: "lthumb", r3: "rthumb", ps: "guide",
  touchpad: "none", mute: "none",
  dpad_up: "dpad_up", dpad_right: "dpad_right", dpad_down: "dpad_down", dpad_left: "dpad_left",
};

const CURVES: [Curve, string][] = [
  ["linear", t("curve.linear")],
  ["precise", t("curve.precise")],
  ["aggressive", t("curve.aggressive")],
];

const NEUTRAL_STICK: StickConfig = { deadzone: 0, outer: 100, curve: "linear" };

const targetLabel = (target: XTarget) => TARGETS.find(([id]) => id === target)?.[1] ?? target;

export function initInputUi(ctx: Ctx) {
  const sticksEl = $("sticks-config");
  const remapEl = $("remap-grid");

  /** The configuration of the profile on display, edited in place then sent whole. */
  const input = (): InputConfig | null => ctx.current()?.profile.input ?? null;
  const save = () => {
    const cfg = input();
    if (cfg) ctx.call("set_input_config", { input: cfg });
  };

  function renderSticks() {
    const cfg = input();
    sticksEl.replaceChildren();
    if (!cfg) return;

    for (const [key, title] of [
      ["left_stick", t("controls.leftStick")],
      ["right_stick", t("controls.rightStick")],
    ] as const) {
      const stick = cfg[key];
      const block = document.createElement("div");
      block.className = "stick-config";

      const heading = document.createElement("h3");
      heading.textContent = title;

      block.append(
        heading,
        makeSlider(t("controls.deadzone"), 0, 50, stick.deadzone, percent, (v) => {
          stick.deadzone = v;
          save();
        }),
        makeSlider(t("controls.outer"), 50, 100, stick.outer, percent, (v) => {
          stick.outer = v;
          save();
        }),
        makeSelect(CURVES, stick.curve, (curve) => {
          stick.curve = curve;
          save();
        }),
      );
      sticksEl.appendChild(block);
    }
  }

  function renderRemap() {
    const cfg = input();
    remapEl.replaceChildren();
    if (!cfg) return;

    BUTTON_IDS.forEach((id, i) => {
      // The last remapping of a button wins (as on the Rust side).
      const override = [...cfg.remap].reverse().find((r) => r.from === id);
      const options: [string, string][] = [
        ["", t("controls.default", { target: targetLabel(DEFAULT_TARGET[id]) })],
        ...TARGETS.map(([target, label]) => [target, label] as [string, string]),
      ];

      const item = document.createElement("div");
      item.className = "remap-item";
      const name = document.createElement("span");
      name.textContent = BUTTON_NAMES[i];

      const select = makeSelect(options, override ? override.to : "", (value) => {
        // Only the differences from the default mapping are kept.
        cfg.remap = cfg.remap.filter((r) => r.from !== id);
        if (value !== "" && value !== DEFAULT_TARGET[id]) cfg.remap.push({ from: id, to: value as XTarget });
        save();
      });
      item.append(name, select);
      remapEl.appendChild(item);
    });
  }

  $("input-reset-sticks").addEventListener("click", () => {
    const cfg = input();
    if (!cfg) return;
    cfg.left_stick = { ...NEUTRAL_STICK };
    cfg.right_stick = { ...NEUTRAL_STICK };
    save();
    renderSticks();
  });

  $("input-reset-remap").addEventListener("click", () => {
    const cfg = input();
    if (!cfg) return;
    cfg.remap = [];
    save();
    renderRemap();
  });

  return {
    load(_profile: Profile) {
      renderSticks();
      renderRemap();
    },
  };
}
