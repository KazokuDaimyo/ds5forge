// The colours of the DualSense, to give the illustration the colour of the controller in your hands.
//
// Each part of the controller can have its own colour in a colourway: the shell (the wings), the buttons
// (face buttons and D-pad), the touchpad, and the plate (central plate, triggers and stick caps). A part left
// out keeps the colour of Sony's artwork. The outlines and the light bar never change.
//
// Sony publishes no colour codes. Except for White, which is Sony's own artwork, the values below were
// measured on the official product photos of each edition: they are the colour the controller shows in
// those photos (lit, with reflections), not Pantone references. Where a touchpad is not listed it follows
// the shell.

export interface Colorway {
  /** Stable identifier, saved with the controller. */
  id: string;
  /** The name Sony gives to the colour. Product names are not translated. */
  name: string;
  /** Colour of the shell. */
  shell: string;
  /** Colour of the face buttons and the D-pad, if it differs from the artwork's. */
  buttons?: string;
  /** Colour of the touchpad, if it differs from the artwork's. */
  touchpad?: string;
  /** Colour of the central plate, the triggers and the stick caps, if it differs from the artwork's (dark). */
  plate?: string;
}

/** The first one is the default: the colour of Sony's artwork. */
export const COLORWAYS: Colorway[] = [
  { id: "white", name: "White", shell: "#e8ecef" },
  { id: "midnight-black", name: "Midnight Black", shell: "#2a2c30", buttons: "#1e1d1f" },
  { id: "cosmic-red", name: "Cosmic Red", shell: "#a31a4b", buttons: "#a6064a" },
  { id: "starlight-blue", name: "Starlight Blue", shell: "#52bdf8", buttons: "#6dc1f8" },
  { id: "nova-pink", name: "Nova Pink", shell: "#ea3f9b", buttons: "#df4093" },
  { id: "galactic-purple", name: "Galactic Purple", shell: "#4a2d9b", buttons: "#4a2b9b" },
  { id: "sterling-silver", name: "Sterling Silver", shell: "#a0a6b8", buttons: "#8c91a5" },
  { id: "cobalt-blue", name: "Cobalt Blue", shell: "#2c31ac", buttons: "#0a2b93" },
  { id: "volcanic-red", name: "Volcanic Red", shell: "#b21434", buttons: "#a2142e" },
  { id: "chroma-pearl", name: "Chroma Pearl", shell: "#e1e4ec", buttons: "#e5e9f2", plate: "#e1e4ec" },
  { id: "alpine-green", name: "Alpine Green", shell: "#07534c", touchpad: "#045d5b", buttons: "#005b55", plate: "#dfe2ea" },
];

/** The colourway with this identifier; the default one if it is unknown or missing. */
export const colorwayOf = (id: string | null | undefined): Colorway =>
  COLORWAYS.find((c) => c.id === id) ?? COLORWAYS[0];
