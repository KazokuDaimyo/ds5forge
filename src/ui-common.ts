// Small interface helpers shared by the modules.

import type { ControllerInfo, Profile, Rgb } from "./types";

export const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

/** What main.ts gives the modules to act on the selected controller. */
export interface Ctx {
  current(): ControllerInfo | null;
  /** Applies a setting to the local copy of the profile (the backend has its own copy). */
  editLocal(fn: (p: Profile) => void): void;
  /** Calls a backend command for the selected controller. */
  call(cmd: string, args?: Record<string, unknown>): void;
}

export const rgbToHex = ([r, g, b]: Rgb): string =>
  "#" + [r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("");

export const hexToRgb = (hex: string): Rgb => [
  parseInt(hex.slice(1, 3), 16),
  parseInt(hex.slice(3, 5), 16),
  parseInt(hex.slice(5, 7), 16),
];

export function makeSelect<T extends string>(
  options: [T, string][],
  value: T,
  onChange: (v: T) => void,
): HTMLSelectElement {
  const select = document.createElement("select");
  for (const [v, label] of options) select.add(new Option(label, v));
  select.value = value;
  select.addEventListener("change", () => onChange(select.value as T));
  return select;
}

/** A slider with its label and its displayed value. `unit` is appended to the value (for example "%"). */
export function makeSlider(
  label: string,
  min: number,
  max: number,
  value: number,
  unit: string,
  onInput: (v: number) => void,
): HTMLLabelElement {
  const wrap = document.createElement("label");
  wrap.className = "slider";
  const input = document.createElement("input");
  input.type = "range";
  input.min = String(min);
  input.max = String(max);
  input.value = String(value);
  const out = document.createElement("output");
  out.textContent = `${value}${unit}`;
  input.addEventListener("input", () => {
    out.textContent = `${input.value}${unit}`;
    onInput(Number(input.value));
  });
  wrap.append(label, input, out);
  return wrap;
}

export function makeSwitch(label: string, checked: boolean, onChange: (v: boolean) => void): HTMLLabelElement {
  const wrap = document.createElement("label");
  wrap.className = "switch";
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = checked;
  input.addEventListener("change", () => onChange(input.checked));
  wrap.append(input, label);
  return wrap;
}
