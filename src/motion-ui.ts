// Gyroscope and touchpad: aiming with the mouse or the right stick, touchpad as a mouse.

import { t } from "./i18n";
import { BUTTON_IDS, BUTTON_NAMES, type GyroMode, type MotionConfig, type Profile, type TouchpadMode } from "./types";
import { $, makeSelect, makeSlider, makeSwitch, type Ctx } from "./ui-common";

const GYRO_MODES: [GyroMode, string][] = [
  ["off", t("motion.off")],
  ["right_stick", t("motion.gyroRightStick")],
  ["mouse", t("motion.gyroMouse")],
];

const TOUCHPAD_MODES: [TouchpadMode, string][] = [
  ["off", t("motion.off")],
  ["mouse", t("motion.touchpadMouse")],
];

/** Gyroscope activation: always, or only while holding a button. */
const ACTIVATIONS: [string, string][] = [
  ["", t("motion.always")],
  ...BUTTON_IDS.map((id, i) => [id, t("motion.onlyWhileHolding", { button: BUTTON_NAMES[i] })] as [string, string]),
];

export function initMotionUi(ctx: Ctx) {
  const gyroEl = $("motion-gyro");
  const touchpadEl = $("motion-touchpad");

  const motion = (): MotionConfig | null => ctx.current()?.profile.motion ?? null;
  const save = () => {
    const cfg = motion();
    if (cfg) ctx.call("set_motion_config", { motion: cfg });
  };

  function render() {
    const cfg = motion();
    gyroEl.replaceChildren();
    touchpadEl.replaceChildren();
    if (!cfg) return;

    // --- Gyroscope ---
    const gyroOff = cfg.gyro === "off";
    const detail = document.createElement("div");
    detail.className = "motion-detail" + (gyroOff ? " dim" : "");
    detail.append(
      makeSlider(t("motion.sensitivity"), 1, 100, cfg.gyro_sensitivity, "", (v) => {
        cfg.gyro_sensitivity = v;
        save();
      }),
      makeSelect(ACTIVATIONS, cfg.gyro_hold ?? "", (value) => {
        cfg.gyro_hold = value === "" ? null : value;
        save();
      }),
      makeSwitch(t("motion.invertX"), cfg.gyro_invert_x, (v) => {
        cfg.gyro_invert_x = v;
        save();
      }),
      makeSwitch(t("motion.invertY"), cfg.gyro_invert_y, (v) => {
        cfg.gyro_invert_y = v;
        save();
      }),
    );
    gyroEl.append(
      makeSelect(GYRO_MODES, cfg.gyro, (mode) => {
        cfg.gyro = mode;
        save();
        render(); // greys the detailed settings in or out
      }),
      detail,
    );

    // --- Touchpad ---
    const padDetail = document.createElement("div");
    padDetail.className = "motion-detail" + (cfg.touchpad === "off" ? " dim" : "");
    padDetail.append(
      makeSlider(t("motion.sensitivity"), 1, 100, cfg.touchpad_sensitivity, "", (v) => {
        cfg.touchpad_sensitivity = v;
        save();
      }),
    );
    touchpadEl.append(
      makeSelect(TOUCHPAD_MODES, cfg.touchpad, (mode) => {
        cfg.touchpad = mode;
        save();
        render();
      }),
      padDetail,
    );
  }

  return {
    load(_profile: Profile) {
      render();
    },
  };
}
