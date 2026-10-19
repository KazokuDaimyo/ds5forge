// Light bar mode: fixed colour, following the battery, or following the mode (native / Xbox 360).

import { t } from "./i18n";
import type { LightbarMode, Profile } from "./types";
import { $, hexToRgb, rgbToHex, type Ctx } from "./ui-common";

const MODES: [LightbarMode, string][] = [
  ["fixed", t("lighting.modeFixed")],
  ["battery", t("lighting.modeBattery")],
  ["mode", t("lighting.modeMode")],
];

const HINTS: Record<LightbarMode, string> = {
  fixed: "",
  battery: t("lighting.hintBattery"),
  mode: t("lighting.hintMode"),
};

export function initLightingUi(ctx: Ctx) {
  const select = $<HTMLSelectElement>("lightbar-mode");
  const colorsRow = $("mode-colors");
  const xbox = $<HTMLInputElement>("color-xbox");
  const native = $<HTMLInputElement>("color-native");
  const fixedColor = $<HTMLInputElement>("color");
  const offButton = $<HTMLButtonElement>("color-off");
  const hint = $("lightbar-hint");

  for (const [mode, label] of MODES) select.add(new Option(label, mode));

  /** The fixed colour picker only makes sense in fixed mode; the mode pickers only in "mode" mode. */
  const refresh = (mode: LightbarMode) => {
    fixedColor.disabled = mode !== "fixed";
    offButton.disabled = mode !== "fixed";
    colorsRow.hidden = mode !== "mode";
    hint.textContent = HINTS[mode];
    hint.hidden = HINTS[mode] === "";
  };

  const save = () => {
    const mode = select.value as LightbarMode;
    const mode_colors = { xbox: hexToRgb(xbox.value), native: hexToRgb(native.value) };
    ctx.editLocal((p) => {
      p.lightbar_mode = mode;
      p.mode_colors = mode_colors;
    });
    ctx.call("set_lightbar_mode", { mode, xbox: mode_colors.xbox, native: mode_colors.native });
    refresh(mode);
  };

  select.addEventListener("change", save);
  xbox.addEventListener("input", save);
  native.addEventListener("input", save);

  return {
    load(profile: Profile) {
      select.value = profile.lightbar_mode;
      xbox.value = rgbToHex(profile.mode_colors.xbox);
      native.value = rgbToHex(profile.mode_colors.native);
      refresh(profile.lightbar_mode);
    },
  };
}
