// DualSense illustration animated with the controller's real state: buttons light up, triggers
// tilt, sticks move, and the touchpad and light bar react.
//
// The drawing is the official artwork from Sony Interactive Entertainment (perspective view, where
// L1/L2/R1/R2 are visible), converted to SVG by `tools/extract-dualsense-art.mjs` (see
// src/assets/README.md). It is not modified: highlight layers are added, the light bar takes the
// controller's colour, and whatever the triggers and sticks used to hide (absent from the drawing)
// is filled in.

import svgSource from "./assets/dualsense.svg?raw";
import "./art.css";

/** What the drawing shows: the same fields as the state events. */
export interface ArtState {
  lx: number;
  ly: number;
  rx: number;
  ry: number;
  l2: number;
  r2: number;
  buttons: number;
}


/**
 * A trigger is fixed by its base (the edge that goes under L1) and it is its free end, the upper
 * arc, that tilts towards the player, as seen from the side on a real controller: a rigid body
 * that pivots. In this top view, a pivot around a horizontal line (the hinge, `data-hinge`: a point
 * and a slope read from the drawing) amounts to shortening the whole trigger vertically towards
 * that line. It therefore keeps its shape, the upper arc comes down, and nothing slides under L1.
 * At full travel, the trigger keeps only `1 - TRIGGER_FOLD` of its height.
 */
const TRIGGER_FOLD = 0.4;

/**
 * Travel of a stick cap at full deflection, in drawing units (its width is 53). It is tuned by eye
 * so that the four directions look equally pronounced, not to be the same everywhere:
 * - at rest, Sony draws the cap 10 units lower than the centre of the well (perspective): upwards,
 *   it first only gets centred, so it has to go further for the movement to be visible;
 * - downwards, it leaves the plate and stands out against the background: little travel is enough.
 */
const STICK_TRAVEL_X = 12;
const STICK_TRAVEL_UP = 14;
const STICK_TRAVEL_DOWN = 8;
/** The dish is higher than the bottom of the cap, so it moves slightly more than the cap. */
const DISH_PARALLAX = 1.12;

let instances = 0;

/** The elements carrying `data-<attribute>`, grouped by the value of that attribute. */
function groupBy(root: Element, attribute: string): Map<string, Element[]> {
  const groups = new Map<string, Element[]>();
  root.querySelectorAll(`[data-${attribute}]`).forEach((node) => {
    const key = node.getAttribute(`data-${attribute}`)!;
    groups.set(key, [...(groups.get(key) ?? []), node]);
  });
  return groups;
}

export function createControllerArt() {
  const el = document.createElement("div");
  el.className = "art-wrap";
  // The identifiers in the SVG contain __ID__: several copies can coexist (overlay).
  el.innerHTML = svgSource.split("__ID__").join(String(instances++));

  // A `data-part="n"` attribute matches bit n of `ControllerState::buttons`
  // (order of `btn` in src-tauri/src/controller.rs).
  const parts = [...groupBy(el, "part")].map(([bit, nodes]) => ({ mask: 1 << Number(bit), nodes }));
  const fills = groupBy(el, "fill");

  // The hinge of each trigger: the point (x, y) and the slope of its line, in drawing coordinates.
  const sinks = groupBy(el, "sink");
  const hinges = new Map<Element, { x: number; y: number; slope: number }>();
  sinks.forEach((nodes) =>
    nodes.forEach((node) => {
      const [x, y, slope] = node.getAttribute("data-hinge")!.split(" ").map(Number);
      hinges.set(node, { x, y, slope });
    }),
  );
  // The trigger folds onto its hinge (see TRIGGER_FOLD) and takes the accent colour with pressure.
  const pressTrigger = (side: "l2" | "r2", value: number) => {
    const ratio = value / 255;
    const k = 1 - ratio * TRIGGER_FOLD;
    sinks.get(side)?.forEach((node) => {
      // Vertical shortening by a factor k towards the line y = h.y + slope * (x - h.x): x does not
      // change, y becomes k * y + (1 - k) * (h.y + slope * (x - h.x)).
      const h = hinges.get(node)!;
      const matrix = [1, (1 - k) * h.slope, 0, k, 0, (1 - k) * (h.y - h.slope * h.x)].map((v) => v.toFixed(4)).join(", ");
      (node as SVGElement).style.transform = `matrix(${matrix})`;
    });
    fills.get(side)?.forEach((node) => ((node as SVGElement).style.opacity = String(ratio * 0.9)));
  };

  const sticks = groupBy(el, "stick");
  const dishes = groupBy(el, "dish");
  const placed = { l: "", r: "" };
  // The cap slides over the plate; the fixed drawing is cut out in its place and what it hid is drawn underneath.
  const moveStick = (side: "l" | "r", rawX: number, rawY: number) => {
    // Controller bytes: 0 to 255, 128 at rest, y downwards as in the drawing.
    let x = (rawX - 128) / 128;
    let y = (rawY - 128) / 128;
    const length = Math.hypot(x, y);
    if (length > 1) {
      // The stick is round: some controllers go beyond the radius in the diagonals.
      x /= length;
      y /= length;
    }
    const dx = x * STICK_TRAVEL_X;
    const dy = y * (y < 0 ? STICK_TRAVEL_UP : STICK_TRAVEL_DOWN);
    const move = (k: number) => `translate(${(dx * k).toFixed(2)}px, ${(dy * k).toFixed(2)}px)`;
    if (move(1) === placed[side]) return;
    placed[side] = move(1);
    sticks.get(side)?.forEach((node) => ((node as SVGElement).style.transform = move(1)));
    dishes.get(side)?.forEach((node) => ((node as SVGElement).style.transform = move(DISH_PARALLAX - 1)));
  };

  return {
    el,

    update(s: ArtState) {
      for (const { mask, nodes } of parts) {
        const on = (s.buttons & mask) !== 0;
        nodes.forEach((node) => node.classList.toggle("on", on));
      }
      pressTrigger("l2", s.l2);
      pressTrigger("r2", s.r2);
      moveStick("l", s.lx, s.ly);
      moveStick("r", s.rx, s.ry);
    },
  };
}
