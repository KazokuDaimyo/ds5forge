// Battery display, shared by the main window and the overlay.
//
// The controller does not report a percentage but a level from 0 to 10. The backend sends the
// middle of the band (level × 10 + 5, 100 at level 10): the level is recovered from it, then shown
// as a band ("80–90%") and as a gauge with 10 segments.

import "./gauge.css";
import { percent as percentSign, t } from "./i18n";

export type Charge = "unknown" | "discharging" | "charging" | "full" | "error";

/** Number of gauge segments = number of levels reported by the controller. */
export const SEGMENTS = 10;

/** At or below this percentage, the battery is considered low. */
const LOW_PERCENT = 20;

/** The level from 0 to 10 that matches the received percentage. */
export const levelOf = (percent: number): number =>
  Math.min(SEGMENTS, Math.max(0, Math.floor(percent / 10)));

/** "80–90%", or "100%" when the battery is full. */
export function rangeText(percent: number): string {
  const level = levelOf(percent);
  return level >= SEGMENTS ? `100${percentSign}` : `${level * 10}–${level * 10 + 10}${percentSign}`;
}

/** Marker appended after the band: charging or charged. */
export const chargeMark = (charge: Charge): string =>
  charge === "charging" ? " ⚡" : charge === "full" ? " ✓" : "";

/** A battery that is charging is never "low", even at 5%. */
export const isLow = (percent: number, charge: Charge): boolean =>
  percent <= LOW_PERCENT && charge !== "charging" && charge !== "full";

/**
 * Gauge with 10 segments: the full segments are certain; the next one is hatched, because the
 * battery is somewhere within the band.
 */
export function createGauge(percent: number, charge: Charge): HTMLElement {
  const level = levelOf(percent);
  const gauge = document.createElement("div");
  gauge.className = "gauge" + (isLow(percent, charge) ? " low" : "");
  gauge.setAttribute("role", "img");
  gauge.setAttribute("aria-label", t("battery.label", { range: rangeText(percent) }));

  for (let i = 0; i < SEGMENTS; i++) {
    const segment = document.createElement("i");
    if (i < level) segment.className = "on";
    else if (i === level) segment.className = "partial";
    gauge.appendChild(segment);
  }
  return gauge;
}
