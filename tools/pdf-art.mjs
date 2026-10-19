// Reads the vector paths of a page of Sony's PDF (shared by the drawing generators).

import fs from "node:fs";
import * as pdfjs from "pdfjs-dist/legacy/build/pdf.mjs";

// Affine algebra (a b c d e f): apply B first, then A.
const mul = (A, B) => [
  A[0] * B[0] + A[2] * B[1], A[1] * B[0] + A[3] * B[1],
  A[0] * B[2] + A[2] * B[3], A[1] * B[2] + A[3] * B[3],
  A[0] * B[4] + A[2] * B[5] + A[4], A[1] * B[4] + A[3] * B[5] + A[5],
];
const apply = (M, x, y) => [M[0] * x + M[2] * y + M[4], M[1] * x + M[3] * y + M[5]];
export const r2 = (n) => Math.round(n * 100) / 100;

/**
 * Every path of the page, in drawing order, with its colour, its bounding box
 * and its SVG path (origin at the top left). A gradient becomes a `kind: "shading"` piece.
 */
export async function readPieces(pdfFile, pageNumber) {
  const doc = await pdfjs.getDocument({ data: new Uint8Array(fs.readFileSync(pdfFile)), verbosity: 0 }).promise;
  const page = await doc.getPage(pageNumber);
  const H = page.getViewport({ scale: 1 }).height;
  const list = await page.getOperatorList();
  const OPS = pdfjs.OPS;

  let state = { ctm: [1, 0, 0, 1, 0, 0], fill: "#000000", stroke: "#000000", lw: 1 };
  const stack = [];
  const pieces = [];
  let pendingClip = false;
  let clipShape = null;

  function toSvgPath(data) {
    // Float32Array: sequences of [code, coordinates…]; 0 moveTo, 1 lineTo, 2 curveTo, 3 quadTo, 4 close.
    const pts = [];
    let d = "";
    const P = (x, y) => {
      const [tx, ty] = apply(state.ctm, x, y);
      pts.push([tx, H - ty]);
      return `${r2(tx)} ${r2(H - ty)}`;
    };
    for (let i = 0; i < data.length; ) {
      const op = data[i++];
      if (op === 0) { d += `M${P(data[i], data[i + 1])}`; i += 2; }
      else if (op === 1) { d += `L${P(data[i], data[i + 1])}`; i += 2; }
      else if (op === 2) { d += `C${P(data[i], data[i + 1])} ${P(data[i + 2], data[i + 3])} ${P(data[i + 4], data[i + 5])}`; i += 6; }
      else if (op === 3) { d += `Q${P(data[i], data[i + 1])} ${P(data[i + 2], data[i + 3])}`; i += 4; }
      else if (op === 4) d += "Z";
      else throw new Error(`unknown path code: ${op}`);
    }
    const xs = pts.map((p) => p[0]);
    const ys = pts.map((p) => p[1]);
    return { d, bbox: pts.length ? [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)].map(r2) : null };
  }

  for (let i = 0; i < list.fnArray.length; i++) {
    const fn = list.fnArray[i];
    const args = list.argsArray[i];
    if (fn === OPS.save) stack.push({ ...state, ctm: [...state.ctm] });
    else if (fn === OPS.restore) state = stack.pop() ?? state;
    else if (fn === OPS.transform) state.ctm = mul(state.ctm, args);
    else if (fn === OPS.setFillRGBColor) state.fill = args[0];
    else if (fn === OPS.setStrokeRGBColor) state.stroke = args[0];
    else if (fn === OPS.setLineWidth) state.lw = args[0];
    else if (fn === OPS.clip || fn === OPS.eoClip) pendingClip = true;
    else if (fn === OPS.constructPath) {
      const [paint, [data]] = args;
      const { d, bbox } = toSvgPath(data);
      if (pendingClip && paint === OPS.endPath) {
        clipShape = { d, bbox };
        pendingClip = false;
      }
      const kind = { [OPS.fill]: "fill", [OPS.eoFill]: "eofill", [OPS.stroke]: "stroke", [OPS.closeStroke]: "stroke",
        [OPS.fillStroke]: "fillstroke", [OPS.eoFillStroke]: "eofillstroke" }[paint];
      if (kind && bbox) {
        pieces.push({ kind, d, bbox, fill: state.fill, stroke: state.stroke, lw: r2(state.lw * Math.hypot(state.ctm[0], state.ctm[1])) });
      }
    } else if (fn === OPS.shadingFill && clipShape) {
      const ir = page.objs.get(args[0]);
      const [p0, p1] = [apply(state.ctm, 0, 0), apply(state.ctm, 1, 0)];
      pieces.push({
        kind: "shading", d: clipShape.d, bbox: clipShape.bbox, stops: ir[3],
        p0: [r2(p0[0]), r2(H - p0[1])], p1: [r2(p1[0]), r2(H - p1[1])],
      });
    }
  }
  return pieces;
}

/** The pieces whose centre lies in `region` = [x0, y0, x1, y1] (points, origin at the top left). */
export function inRegion(pieces, [x0, y0, x1, y1]) {
  return pieces.filter((p) => {
    const cx = (p.bbox[0] + p.bbox[2]) / 2;
    const cy = (p.bbox[1] + p.bbox[3]) / 2;
    return cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1;
  });
}

/** XML attributes: { a: 1, b: "x" } -> ` a="1" b="x"`. */
export const attrs = (obj = {}) => Object.entries(obj).map(([k, v]) => ` ${k}="${v}"`).join("");

/** Finds the pieces of a colour whose box matches, to within `tolerance` points. */
export function makeFinder(candidates) {
  const claimed = new Set();
  const near = (a, b, tolerance) => a.every((v, i) => Math.abs(v - b[i]) <= tolerance);
  return function find(color, bbox, tolerance = 2.5) {
    const hits = candidates.filter((p) => !claimed.has(p) && p.fill === color && near(p.bbox, bbox, tolerance));
    if (hits.length !== 1) throw new Error(`${hits.length} piece(s) for ${color} ${bbox}`);
    claimed.add(hits[0]);
    return hits[0];
  };
}

/** Bounding box of the drawing, with a margin. */
export function viewBoxOf(pieces, pad = 8) {
  const boxes = pieces.map((p) => p.bbox);
  const [minX, minY, maxX, maxY] = [
    Math.min(...boxes.map((b) => b[0])) - pad, Math.min(...boxes.map((b) => b[1])) - pad,
    Math.max(...boxes.map((b) => b[2])) + pad, Math.max(...boxes.map((b) => b[3])) + pad,
  ].map(r2);
  return `${minX} ${minY} ${r2(maxX - minX)} ${r2(maxY - minY)}`;
}
