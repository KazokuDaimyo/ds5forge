// Generates the animated DualSense illustration from Sony's official PDF.
//
//   npm install --no-save pdfjs-dist          (once, in a working folder)
//   node extract-dualsense-art.mjs "<DualSense_Wireless_Controller_RGB_012.pdf>" src/assets/dualsense.svg
//
// The script reads the perspective view (page 5 of the PDF, top right: L1/L2/R1/R2 are visible on the
// shoulders), extracts each path separately, locates the buttons from their position and colour,
// and writes an SVG where every animatable part carries a `data-part` attribute (button bit),
// `data-fill`, `data-sink` or `data-stick` / `data-dish` (stick caps). The styling of the states is in
// src/art.css; the animation logic is in src/controller-art.ts.
//
// The original artwork belongs to Sony Interactive Entertainment: it is not modified, only highlight
// layers are added on top, and the light bar takes the controller's colour. So that the triggers and the
// sticks can move, what Sony does not draw (what lies beneath) is completed by extending the visible
// paths. The PDF is not included in the repository.

import fs from "node:fs";
import { attrs, inRegion, makeFinder, r2, readPieces, viewBoxOf } from "./pdf-art.mjs";

const [pdfFile, outFile] = process.argv.slice(2);
if (!pdfFile || !outFile) {
  console.error("Usage: node extract-dualsense-art.mjs <file.pdf> <output.svg>");
  process.exit(1);
}

const PAGE = 5; // 8 views on a transparent background; the perspective view is at the top right
const REGION = [1620, 290, 2110, 590]; // in points, origin at the top left

const pieces = await readPieces(pdfFile, PAGE);
const view = inRegion(pieces, REGION);
console.log(`${pieces.length} paths on the page, ${view.length} in the view`);

// ---------------------------------------------------------------------------------------------
// 1. Locating the animatable parts (position and colour, tolerance of 2.5 pt)
// ---------------------------------------------------------------------------------------------

// Colours of Sony's data sheet.
const CLEAR_BUTTON = "#dee2e5";
const LINE_GREY = "#787878";
const NAVY = "#111b25";

const find = makeFinder(view);
const tag = (piece, a) => Object.assign(piece, { attrs: a });

// bit: 0 square, 1 cross, 2 circle, 3 triangle, 4 L1, 5 R1, 6 L2, 7 R2, 8 create, 9 options,
//       10 L3, 11 R3, 12 PS, 13 touchpad, 14 mic, 15 up, 16 right, 17 down, 18 left
// Each button is drawn in two layers (base and top), plus a symbol and an outline.
const button = (bit, [base, top], glyph, outline) => {
  tag(find(CLEAR_BUTTON, base), { class: "btn-fill", "data-part": bit });
  tag(find(CLEAR_BUTTON, top), { class: "btn-fill", "data-part": bit });
  tag(find(LINE_GREY, glyph), { class: "btn-glyph", "data-part": bit });
  tag(find(LINE_GREY, outline), { class: "btn-outline" });
};
button(3, [[2001, 406, 2031, 433], [1999, 403, 2031, 426]], [2005, 410, 2025, 425], [1999, 403, 2031, 433]); // triangle
button(2, [[2032, 433, 2063, 463], [2031, 430, 2064, 454]], [2038, 438, 2060, 457], [2031, 431, 2064, 461]); // circle
button(1, [[1998, 465, 2029, 489], [1997, 462, 2029, 479]], [2005, 470, 2022, 483], [1997, 462, 2029, 489]); // cross
button(0, [[1965, 437, 1995, 461], [1964, 434, 1996, 449]], [1972, 441, 1988, 455], [1964, 434, 1996, 461]); // square
button(15, [[1709, 419, 1735, 444], [1709, 414, 1735, 425]], [1719, 422, 1727, 426], [1709, 413, 1736, 444]); // up
button(17, [[1710, 454, 1736, 480], [1710, 451, 1736, 474]], [1719, 471, 1726, 474], [1709, 450, 1736, 481]); // down
button(18, [[1684, 439, 1716, 460], [1684, 435, 1716, 449]], [1690, 446, 1694, 453], [1684, 435, 1716, 460]); // left
button(16, [[1729, 439, 1761, 460], [1729, 435, 1761, 449]], [1752, 446, 1756, 452], [1729, 435, 1761, 460]); // right

// PS logo: three pieces
for (const box of [[1866, 484, 1878, 500], [1872, 494, 1885, 500], [1854, 493, 1865, 498]]) {
  tag(find(LINE_GREY, box), { class: "btn-glyph", "data-part": 12 });
}

// Shoulders: a single dark piece per side (background of L1 and L2).
const shoulders = { l2: find(NAVY, [1689, 314, 1756, 379]), r2: find(NAVY, [1981, 314, 2048, 379]) };
tag(shoulders.l2, {});
tag(shoulders.r2, {});



// L2 / R2 labels: they follow the trigger when it is pressed (they are drawn with it).
const labelsL2 = [[1738, 329, 1742, 332], [1743, 330, 1748, 332]].map((box) => find(LINE_GREY, box));
const labelsR2 = [[1989, 330, 1995, 332], [1996, 329, 2000, 332]].map((box) => find(LINE_GREY, box));

// The outlines (shoulders, sticks, touchpad…) form a single piece made of closed subpaths.
// They are found by their box: they serve as shapes for the highlights and the movement.
const compound = find(LINE_GREY, [1642, 314, 2095, 568], 0.5);
const subpaths = compound.d.split(/(?=M)/).map((d) => {
  const n = [...d.matchAll(/(-?\d+(?:\.\d+)?) (-?\d+(?:\.\d+)?)/g)].map((m) => [Number(m[1]), Number(m[2])]);
  const xs = n.map((p) => p[0]);
  const ys = n.map((p) => p[1]);
  return { d, bbox: [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)] };
});
function pick(box, tolerance = 1.5) {
  const hits = subpaths.filter((s) => s.bbox.every((v, i) => Math.abs(v - box[i]) <= tolerance));
  if (hits.length !== 1) throw new Error(`${hits.length} outline(s) for ${box}`);
  return hits[0];
}
const shape = (box, tolerance) => pick(box, tolerance).d;

const SHAPES = {
  touchpad: shape([1773, 363, 1964, 459]),
  l2: { l: shape([1697, 314, 1753, 344]), r: shape([1984, 314, 2040, 344]) },
  l1: { l: shape([1691, 340, 1751, 367]), r: shape([1986, 339, 2046, 367]) },
};

// ---------------------------------------------------------------------------------------------
// Sticks. Sony only draws what is visible: the cap (outer ellipse) and its dish (inner ellipse)
// hide part of three rings and of the edge of the plate, which therefore do not exist in the
// drawing. So that the cap can move, it is separated from the fixed drawing (cut out in its place) and
// what it used to hide is drawn underneath:
//   - the edge of the plate: the outline of the plate goes around the bottom of the cap, which sticks out; it is
//     replaced by an edge joined smoothly to the two visible arcs on either side;
//   - the missing arcs of the rings: ellipses that extend the visible arcs.
// ---------------------------------------------------------------------------------------------

/** Ellipse (cx, cy, rx, ry) of an outline that is one, from its box. */
const ellipseOf = ([x0, y0, x1, y1]) => ({ cx: (x0 + x1) / 2, cy: (y0 + y1) / 2, rx: (x1 - x0) / 2, ry: (y1 - y0) / 2 });

const HAIRLINE = 0.24; // ordinary hairline of the drawing, set against the outside of the shape
// Every line of the drawing is drawn BOLDNESS times thicker than in Sony's artwork (strokes here, the lines drawn
// as filled shapes in art.css), so that they stay readable at the size of the interface.
const BOLDNESS = 1;
const line = (w) => r2(w * BOLDNESS);
const SILHOUETTE = 0.48; // silhouette stroke (edge of the plate, bottom of the cap), set against the outside
// The carve-out removes everything the cap has around it. Its margin comfortably exceeds the silhouette stroke:
// at the scale of the application (1.45 px per unit), the anti-aliasing of the carve-out edge would otherwise
// leave a pale arc where the cap used to be.
const CARVE = SILHOUETTE * BOLDNESS + 0.8;

const plate = find(NAVY, [1690, 438, 2046, 567]); // the dark plate around the sticks
const STICKS = {
  l: {
    cap: pick([1768, 488, 1821, 534]),
    dish: find(LINE_GREY, [1776, 500, 1813, 529]),
    // Rings of the well, from the outside inwards: [cx, cy, rx, ry] of the centre line of the hairline.
    // Each Sony hairline is the gap (0.24) between two neighbouring outlines; these ellipses are fitted to
    // their visible arcs (gap of 0.01 to 0.25 depending on the ring) and extend those that the cap used to hide.
    rings: [[1794.6, 500.9, 35.01, 29.65], [1794.78, 501.55, 29.85, 25.57], [1794.8, 502.34, 27.44, 24.0]],
  },
  r: {
    cap: pick([1916, 489, 1969, 534]),
    dish: find(LINE_GREY, [1924, 500, 1961, 529]),
    rings: [[1942.53, 501.1, 34.9, 29.22], [1942.43, 500.39, 29.82, 24.24], [1942.43, 500.88, 27.51, 22.33]],
  },
};

/** The M / L / C / Z commands of a path, in absolute coordinates. */
const commands = (d) =>
  [...d.matchAll(/([MLCZ])([^MLCZ]*)/g)].map(([, c, n]) => ({ c, n: (n.match(/-?\d+(?:\.\d+)?/g) ?? []).map(Number) }));
const commandsToPath = (cmds) => cmds.map(({ c, n }) => c + n.map(r2).join(" ")).join("");
const endOf = (cmd) => cmd.n.slice(-2);

/**
 * The stretch of the plate outline that runs along the cap, and the edge to put in its place. The edge is
 * a Hermite curve: same point and same tangent as the two visible arcs around it.
 * Also returns the complete curve (left arc, edge, right arc) to draw the silhouette.
 */
function plateEdge(cmds, cap) {
  const { cx, cy, rx, ry } = ellipseOf(cap.bbox);
  const onCap = ([x, y]) => Math.abs(Math.hypot((x - cx) / rx, (y - cy) / ry) - 1) * Math.min(rx, ry) < 0.5;
  let at = null;
  const segs = cmds.map((cmd, i) => {
    const start = at;
    if (cmd.c !== "Z") at = endOf(cmd);
    return { cmd, i, start, end: at };
  });
  const run = segs.filter((g) => g.start && g.cmd.c !== "M" && g.cmd.c !== "Z" && onCap(g.start) && onCap(g.end));
  if (!run.length || run.some((g, k) => g.i !== run[0].i + k)) throw new Error("the plate outline does not run along the cap in one piece");
  const [prev, next] = [segs[run[0].i - 1], segs[run.at(-1).i + 1]];
  if (prev.cmd.c !== "C" || next.cmd.c !== "C") throw new Error("the cap is not surrounded by two arcs of the plate");

  const unit = ([x, y]) => [x / Math.hypot(x, y), y / Math.hypot(x, y)];
  const [from, to] = [prev.end, next.start];
  const t1 = unit([from[0] - prev.cmd.n[2], from[1] - prev.cmd.n[3]]); // exit of the first arc
  const t2 = unit([next.cmd.n[0] - to[0], next.cmd.n[1] - to[1]]); // entry of the second
  const h = Math.hypot(to[0] - from[0], to[1] - from[1]) / 3;
  const hidden = { c: "C", n: [from[0] + t1[0] * h, from[1] + t1[1] * h, to[0] - t2[0] * h, to[1] - t2[1] * h, to[0], to[1]] };
  const edge = commandsToPath([{ c: "M", n: prev.start }, prev.cmd, hidden, next.cmd]);
  return { first: prev.i + 1, last: run.at(-1).i, hidden, edge };
}

/** The outline of a piece (the white body, the plate) without the detour around the caps, and the edge on each side. */
function withoutCaps(piece) {
  const cmds = commands(piece.d);
  const edits = ["l", "r"].map((side) => ({ side, ...plateEdge(cmds, STICKS[side].cap) })).sort((x, y) => y.first - x.first);
  for (const e of edits) cmds.splice(e.first, e.last - e.first + 1, e.hidden);
  return { d: commandsToPath(cmds), edge: Object.fromEntries(edits.map((e) => [e.side, e.edge])) };
}
// The white body follows the same edge as the plate (it extends beyond it by less than a tenth): both go around the caps.
const bodyPiece = find("#e8ecef", [1643, 314, 2095, 568]);
const bodyShape = withoutCaps(bodyPiece);
const plateShape = withoutCaps(plate);
const plateD = plateShape.d;
// The two line paths that touch the caps (ring outlines, silhouette stroke): they are cut out in their
// place, the rest of the drawing has nothing to lose there.
const stickLines = [compound, find(LINE_GREY, [1642, 313, 2095, 568], 1)];

// ---------------------------------------------------------------------------------------------
// 2. Assembling the SVG
// ---------------------------------------------------------------------------------------------

const path = (a, d, extra = "") => `<path${attrs(a)} d="${d}"${extra}/>`;
const asPath = (p) => path(p.attrs ?? {}, p.d, ` fill="${p.fill}"${p.kind.includes("stroke") ? ` stroke="${p.stroke}" stroke-width="${line(p.lw)}"` : ""}`);

/**
 * The hinge of a trigger: the line along its lower edge, where it goes under L1. A trigger is fixed by
 * its base and it is its free end (the upper arc) that tilts towards the player: seen from above, it
 * shortens towards this line. Returns { x, y, slope } (the line through (x, y)).
 */
function hingeOf(d) {
  // points of the outline (sampled Bézier curves)
  const pts = [];
  let at = null;
  for (const { c, n } of commands(d)) {
    if (c === "M" || c === "L") {
      at = n;
      pts.push(at);
    } else if (c === "C") {
      for (let k = 1; k <= 200; k++) {
        const t = k / 200;
        const u = 1 - t;
        pts.push([0, 1].map((i) => u * u * u * at[i] + 3 * u * u * t * n[i] + 3 * u * t * t * n[2 + i] + t * t * t * n[4 + i]));
      }
      at = n.slice(4);
    }
  }
  // lower edge: the lowest point of each 2-unit slice, without the ends (where the outline rises)
  const xs = pts.map((p) => p[0]);
  const [x0, x1] = [Math.min(...xs), Math.max(...xs)];
  const edge = [];
  for (let x = x0 + 0.1 * (x1 - x0); x < x1 - 0.1 * (x1 - x0); x += 2) {
    const column = pts.filter((p) => Math.abs(p[0] - x) < 1);
    if (column.length) edge.push([x, Math.max(...column.map((p) => p[1]))]);
  }
  // least-squares line y = y0 + slope * (x - x0)
  const n = edge.length;
  const mx = edge.reduce((s, p) => s + p[0], 0) / n;
  const my = edge.reduce((s, p) => s + p[1], 0) / n;
  const slope = edge.reduce((s, p) => s + (p[0] - mx) * (p[1] - my), 0) / edge.reduce((s, p) => s + (p[0] - mx) ** 2, 0);
  return { x: mx, y: my, slope };
}

let defs = "";
let caps = ""; // triggers: drawn under the rest, which is cut out in their place
let body = "";
const wallCarves = []; // cut-outs of the fixed drawing: the top of the side walls of the triggers (see cap)
/**
 * A trigger (L2 or R2). It carries a copy of Sony's own drawing: the fixed drawing (id art) is reused there, cropped to
 * the shape of the trigger plus a margin of one unit (the area carved out of the fixed drawing, see the mask cut) and
 * to the top of its side wall. At rest the result is Sony's drawing, line for line, with no seam; pressed, the
 * trigger folds with its outline, its silhouette and its wall. The copy is slightly larger than the cut-out so
 * that the two overlap. On top: the fill according to the pressure, the click highlight and the label.
 *
 * The side wall (a thin band, located by `wall`) runs along the trigger: above the hinge it is part of the trigger
 * and has to fold with it, otherwise its top would stay upright at the old position; the rest belongs to L1.
 */
function cap(side, bit, trigger, labels, wall) {
  const d = SHAPES.l2[side];
  const hinge = hingeOf(d);
  const [wx0, wy0, wx1] = wall.bbox;
  const cutY = hinge.y + hinge.slope * ((wx0 + wx1) / 2 - hinge.x);
  const box = (bottom) => `x="${r2(wx0 - 1.5)}" y="${r2(wy0 - 2)}" width="${r2(wx1 - wx0 + 3)}" height="${r2(bottom - (wy0 - 2))}"`;
  wallCarves.push(`<rect ${box(cutY)} fill="#000"/>`);
  const [sx0, sy0, sx1, sy1] = shoulders[trigger].bbox;
  defs +=
    `<mask id="part-${trigger}-__ID__" maskUnits="userSpaceOnUse" x="${r2(sx0 - 3)}" y="${r2(sy0 - 3)}" width="${r2(sx1 - sx0 + 6)}" height="${r2(sy1 - sy0 + 6)}">` +
    `<path d="${d}" fill="#fff" stroke="#fff" stroke-width="2.3" stroke-linejoin="round"/><rect ${box(cutY + 1)} fill="#fff"/></mask>`;
  caps +=
    `<g class="trigger-cap" data-sink="${trigger}" data-hinge="${r2(hinge.x)} ${r2(hinge.y)} ${r2(hinge.slope)}">` +
    `<g mask="url(#part-${trigger}-__ID__)"><use href="#art-__ID__"/></g>` +
    `<path class="trigger-fill" opacity="0" data-fill="${trigger}" d="${d}"/>` +
    `<path class="hl" opacity="0" data-part="${bit}" d="${d}"/>` +
    labels.map(asPath).join("") +
    `</g>`;
}
cap("l", 6, "l2", labelsL2, pick([1751, 322.7, 1756.2, 358.7]));
cap("r", 7, "r2", labelsR2, pick([1980.8, 325.2, 1984.4, 358]));

// Sticks. From bottom to top: the body, the silhouette stroke of the plate edge, the plate, the complete rings,
// the rest of the drawing (including the lines cut out where the caps are), the silhouette stroke of the cap
// and the cap. Silhouette and rings only show through the carve-out (where the cap used to be).
const ell = (cx, cy, rx, ry, extra = "") => `<ellipse cx="${r2(cx)}" cy="${r2(cy)}" rx="${r2(rx)}" ry="${r2(ry)}"${extra}/>`;
let rim = "";
let wells = "";
let halos = "";
let sticks = "";
for (const side of ["l", "r"]) {
  const { cap, dish, rings } = STICKS[side];
  const { cx, cy, rx, ry } = ellipseOf(cap.bbox);
  defs += `<clipPath id="carve-${side}-__ID__">${ell(cx, cy, rx + CARVE, ry + CARVE)}</clipPath>`;
  const clipped = `clip-path="url(#carve-${side}-__ID__)" fill="none" stroke="${LINE_GREY}"`;
  // The silhouette stroke is set against the outside of the edge: it is drawn twice as wide, and the plate
  // (drawn on top) hides its inner half.
  rim += `<path ${clipped} stroke-width="${line(SILHOUETTE * 2)}" d="${plateShape.edge[side]}"/>`;
  wells += `<g ${clipped} stroke-width="${line(HAIRLINE)}">${rings.map((e) => ell(...e)).join("")}</g>`;

  // The bottom of the cap has a silhouette stroke where it sticks out of the plate (see the plate-__ID__ mask).
  const halo = ` fill="none" stroke="${LINE_GREY}" stroke-width="${line(SILHOUETTE)}"`;
  halos += `<g class="stick" data-stick="${side}">${ell(cx, cy, rx + SILHOUETTE / 2, ry + SILHOUETTE / 2, halo)}</g>`;
  // The background and hairline of the cap are opaque: they hide the rings it covers.
  // The dish is slightly higher than the bottom of the cap: it moves more (data-dish).
  const outline = ` class="dark" fill="${NAVY}" stroke="${LINE_GREY}" stroke-width="${line(HAIRLINE)}"`;
  sticks +=
    `<g class="stick" data-stick="${side}">` +
    ell(cx, cy, rx + HAIRLINE / 2, ry + HAIRLINE / 2, outline) +
    `<g class="dish" data-dish="${side}">${asPath(dish)}</g>` +
    // Stick click (L3 / R3): the cap lights up
    `<path class="hl" opacity="0" data-part="${side === "l" ? 10 : 11}" d="${cap.d}"/>` +
    `</g>`;
}

for (const p of view) {
  if (labelsL2.includes(p) || labelsR2.includes(p) || p === STICKS.l.dish || p === STICKS.r.dish) continue;
  // The body and the plate are redrawn without the detour around the caps; the silhouette stroke of the
  // edge of the plate goes between the two (the plate hides its inner half), the complete rings just above.
  if (p === bodyPiece) {
    // The `shell` class marks the colour of the controller, which the interface can change.
    // The touchpad belongs to the same path as the shell, but it keeps its colour when the shell changes: it is
    // drawn again, in the original colour, right above it.
    body += `<path class="shell" d="${bodyShape.d}" fill="${p.fill}"/><path class="touchpad-face" d="${SHAPES.touchpad}" fill="${p.fill}"/>${rim}`;
    continue;
  }
  if (p === plate) {
    body += `<path class="dark" d="${plateD}" fill="${p.fill}"/>${wells}`;
    continue;
  }
  if (stickLines.includes(p)) {
    body += `<g mask="url(#sticks-__ID__)"><path d="${p.d}" fill="${p.fill}"/></g>`;
    continue;
  }
  if (p.kind === "shading") {
    // Light bar: same proportions as the original, but the colour follows the controller (--led).
    defs += `<linearGradient id="lightbar-__ID__" gradientUnits="userSpaceOnUse" x1="${p.p0[0]}" y1="${p.p0[1]}" x2="${p.p1[0]}" y2="${p.p1[1]}">` +
      `<stop offset="0" stop-color="#393a39"/><stop offset="0.19" stop-color="#383a3b"/>` +
      `<stop offset="0.49" style="stop-color: color-mix(in srgb, var(--led, #1b4493) 62%, #000)"/>` +
      `<stop offset="0.81" style="stop-color: var(--led, #01509d)"/>` +
      `<stop offset="0.95" style="stop-color: color-mix(in srgb, var(--led, #1b4493) 62%, #000)"/>` +
      `<stop offset="1" style="stop-color: color-mix(in srgb, var(--led, #1b4493) 62%, #000)"/></linearGradient>`;
    body += `<path class="lightbar" d="${p.d}" fill="url(#lightbar-__ID__)"/>`;
    continue;
  }

  const rule = p.kind.startsWith("eo") ? ' fill-rule="evenodd"' : "";
  if (p.kind === "stroke") {
    body += `<path${attrs(p.attrs)} d="${p.d}" fill="none" stroke="${p.stroke}" stroke-width="${line(p.lw)}"/>`;
  } else {
    const strokeAttrs = p.kind.includes("stroke") ? ` stroke="${p.stroke}" stroke-width="${line(p.lw)}"` : "";
    // The dark parts (plate, triggers, stick caps) can take another colour: some editions have them white.
    const dark = p.fill === NAVY ? ' class="dark"' : "";
    body += `<path${dark}${attrs(p.attrs)} d="${p.d}" fill="${p.fill}"${rule}${strokeAttrs}/>`;
  }
}

/**
 * Shape of a button or key, recovered from Sony's drawing: its outline piece is made of several nested
 * subpaths. The largest is the silhouette (which alone only covers the wall of the button) and the next one
 * the face: together they cover the whole button. The highlight thus follows the real shape (here tilted
 * by the perspective), not an approximate rectangle.
 */
function buttonShape(box, tolerance = 2.5) {
  const piece = view.find((p) => p.fill === LINE_GREY && p.bbox.every((v, i) => Math.abs(v - box[i]) <= tolerance));
  if (!piece) throw new Error(`outline not found for ${box}`);
  const area = (d) => {
    const n = [...d.matchAll(/(-?d+(?:.d+)?) (-?d+(?:.d+)?)/g)].map((m) => [Number(m[1]), Number(m[2])]);
    const xs = n.map((p) => p[0]);
    const ys = n.map((p) => p[1]);
    return (Math.max(...xs) - Math.min(...xs)) * (Math.max(...ys) - Math.min(...ys));
  };
  return piece.d.split(/(?=M)/).sort((a, b) => area(b) - area(a)).slice(0, 2);
}

// Highlights added on top, with the exact shape of the drawing when it has one.
const overlay = (bit, shape) =>
  Array.isArray(shape)
    ? `<g class="hl" opacity="0" data-part="${bit}">${shape.map((d) => `<path d="${d}"/>`).join("")}</g>`
    : shape.startsWith("M")
      ? `<path class="hl" opacity="0" data-part="${bit}" d="${shape}"/>`
      : `<${shape} class="hl" opacity="0" data-part="${bit}"/>`;
body +=
  overlay(13, SHAPES.touchpad) +
  overlay(4, SHAPES.l1.l) +
  overlay(5, SHAPES.l1.r) +
  overlay(8, buttonShape([1751, 392, 1766, 413])) + // Create
  overlay(9, buttonShape([1972, 392, 1986, 413])) + // Options
  overlay(14, buttonShape([1857, 508, 1882, 513])) + // Mic

  overlay(12, 'ellipse cx="1869" cy="493" rx="19" ry="15"');

// The whole fixed drawing is cut out where the triggers are: they are redrawn there (see cap),
// and leave a gap when they are pressed. The cut-out (2 pt stroke) is covered by the halo
// of the trigger (2.2 pt), so that no gap shows at rest.
// The lines that touch the stick caps (stickLines) are cut out in the same way: the caps
// are redrawn on top, and what they used to hide underneath.
const [vx, vy, vw, vh] = viewBoxOf(view, 20).split(" ").map(Number);
const region = `maskUnits="userSpaceOnUse" x="${vx}" y="${vy}" width="${vw}" height="${vh}"`;
const whole = `<rect x="${vx}" y="${vy}" width="${vw}" height="${vh}" fill="#fff"/>`;
const carved = ["l", "r"].map((side) => {
  const { cx, cy, rx, ry } = ellipseOf(STICKS[side].cap.bbox);
  return ell(cx, cy, rx + CARVE, ry + CARVE, ' fill="#000"');
});
defs +=
  `<mask id="cut-__ID__" ${region}>${whole}` +
  [SHAPES.l2.l, SHAPES.l2.r].map((d) => `<path d="${d}" fill="#000" stroke="#000" stroke-width="2" stroke-linejoin="round"/>`).join("") +
  wallCarves.join("") +
  `</mask>` +
  `<mask id="sticks-__ID__" ${region}>${whole}${carved.join("")}</mask>` +
  // The silhouette stroke of the cap only exists where it sticks out of the plate.
  `<mask id="plate-__ID__" ${region}>${whole}<path d="${plateD}" fill="#000"/></mask>`;

const svg =
  `<svg class="art" xmlns="http://www.w3.org/2000/svg" viewBox="${viewBoxOf(view)}" ` +
  `role="img" aria-label="DualSense controller"><defs>${defs}</defs>${caps}` +
  `<g mask="url(#cut-__ID__)"><g id="art-__ID__">${body}</g></g>` +
  `<g mask="url(#plate-__ID__)">${halos}</g>${sticks}</svg>\n`;

fs.writeFileSync(outFile, svg);
console.log(`${outFile}: ${(svg.length / 1024).toFixed(1)} KB`);
