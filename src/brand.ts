// Brand logos (Xbox, PlayStation) used as icons in the overlay.
//
// The logos are SVG files in `src/assets/brand/` (see the README there). They are inlined, sized, and
// drawn in the text colour like the other icons, whatever colour they were saved in. This module is
// pure: the files themselves are loaded by `icons.ts`.

/**
 * Prepares the markup of an SVG file to be used as an inline icon: a fixed size, the text colour
 * instead of the colours of the file, and nothing but the `<svg>` element (no XML header, comment or
 * title). Returns `null` if the text is not an SVG.
 */
export function normalizeSvg(raw: string, size: number): string | null {
  const match = /<svg\b([^>]*)>([\s\S]*?)<\/svg>/i.exec(raw);
  if (!match) return null;
  const [, rootAttributes, body] = match;

  // A line drawing (fill="none" on the root) must stay one: only its strokes take the text colour.
  const rootFill = /\sfill\s*=\s*["']([^"']*)["']/i.exec(rootAttributes)?.[1];
  const keep = rootAttributes.replace(/\s(?:width|height|fill|class|style)\s*=\s*(?:"[^"]*"|'[^']*')/gi, "");
  const tinted = body.replace(
    /\b(fill|stroke)\s*=\s*(["'])(?!none\2|currentColor\2|url\()[^"']*\2/gi,
    '$1="currentColor"',
  );
  const fill = rootFill === "none" ? "none" : "currentColor";
  // Without a view box the drawing would not scale to the size.
  const sized = /\sviewBox\s*=/i.test(keep) ? keep : keep + ' viewBox="0 0 24 24"';
  return `<svg${sized} class="icon brand" width="${size}" height="${size}" fill="${fill}" aria-hidden="true">${tinted}</svg>`;
}
