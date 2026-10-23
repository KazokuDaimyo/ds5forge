# Controller artwork

`dualsense.svg` is the perspective view of the official artwork of the **DualSense Wireless Controller**,
provided by Sony Interactive Entertainment in its developer kit
(`DualSense_Wireless_Controller_RGB_012.pdf`, page 5, top-right view: L1/L2/R1/R2 are visible there).

The PDF was converted to SVG by [`tools/extract-dualsense-art.mjs`](../../tools/extract-dualsense-art.mjs)
(which relies on `tools/pdf-art.mjs`). The original drawing is not modified (shapes and colours of Sony's
data sheet). The following is added on top:

- `data-part` / `data-fill` / `data-sink` / `data-hinge` / `data-stick` / `data-dish` attributes to animate
  the buttons, the triggers (`data-hinge`: their hinge, read from the drawing) and the sticks;
- the light bar, whose gradient uses the controller's colour (`--led`);
- **the side wall of the triggers**: the thin band along L2 / R2 belongs to the trigger above its hinge.
  So that it folds together with the trigger (otherwise its top would stay upright at the old position),
  this area is cut out of the fixed drawing and redrawn inside the trigger, from the fixed drawing itself;
- the `shell` class on the body, so that the interface can recolour the controller (see `src/colorways.ts`); the
  touchpad, which belongs to the same path in Sony's drawing, is drawn again above it so that it can have its own
  colour (`touchpad-face`); the dark parts (central plate, triggers, stick caps) carry the `dark` class, since some
  editions have them white;
- highlights for the parts without a shape of their own (L1/L2/R1/R2, create, options, mic, touchpad, stick clicks);
- **what the sticks hide**. Sony only draws what is visible: under each cap, part of the three rings of the
  well and of the edge of the plate is missing. So that the caps can move, the script cuts them out of the
  fixed drawing (as for L2/R2), redraws them on top (opaque background, outline, dish, silhouette stroke where
  they stick out of the plate) and completes what lies beneath: the rings are extended by ellipses fitted on
  their visible arcs, and the edge of the plate is joined smoothly to the two visible arcs on either side
  (same point, same tangent). The exact shape of this hidden edge is invented; at rest, the drawing is
  identical to Sony's (to within a one-pixel hairline at ×8).

## Before redistributing the application

Sony's kit contained **no licence text** (only a data sheet: dimensions, stroke widths, colours). For
personal use this is not an issue; if you publish the application, check the terms of use of the
PlayStation artwork, which may restrict its use.

DualSense and PlayStation are trademarks of Sony Interactive Entertainment. This project is not affiliated with Sony.

## Regenerating

```bash
mkdir work && cd work && npm init -y && npm install pdfjs-dist
node ../tools/extract-dualsense-art.mjs "<path to the PDF>" ../src/assets/dualsense.svg
```

`dualsense.svg` is a generated file: regenerate it after editing the script.
