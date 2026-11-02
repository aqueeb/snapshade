// Inline SVG icons for the two toolbars (24×24 viewBox, stroke = currentColor).
const svg = (body: string): string =>
  `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

export const ICONS = {
  pen: svg('<path d="M4 20l4-1 10-10-3-3L5 16l-1 4z"/><path d="M14 7l3 3"/>'),
  line: svg('<path d="M5 19L19 5"/>'),
  arrow: svg('<path d="M5 19L19 5"/><path d="M11 5h8v8"/>'),
  rect: svg('<rect x="4" y="6" width="16" height="12" rx="1"/>'),
  ellipse: svg('<ellipse cx="12" cy="12" rx="8" ry="6"/>'),
  marker: svg('<path d="M6 19h12"/><path d="M8 15l8-8 2 2-8 8H8v-2z" fill="currentColor" fill-opacity="0.35"/>'),
  blur: svg(
    '<rect x="4" y="4" width="5" height="5" fill="currentColor" stroke="none"/><rect x="15" y="4" width="5" height="5" fill="currentColor" stroke="none" opacity="0.5"/><rect x="9.5" y="9.5" width="5" height="5" fill="currentColor" stroke="none" opacity="0.7"/><rect x="4" y="15" width="5" height="5" fill="currentColor" stroke="none" opacity="0.5"/><rect x="15" y="15" width="5" height="5" fill="currentColor" stroke="none"/>',
  ),
  text: svg('<path d="M5 6h14"/><path d="M12 6v13"/><path d="M9 19h6"/>'),
  undo: svg('<path d="M9 14L4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12h-3"/>'),
  print: svg('<rect x="6" y="3" width="12" height="6"/><rect x="3" y="9" width="18" height="8" rx="1"/><rect x="7" y="14" width="10" height="7"/>'),
  copy: svg('<rect x="9" y="9" width="12" height="12" rx="1"/><path d="M5 15V4a1 1 0 0 1 1-1h10"/>'),
  save: svg('<path d="M12 3v12"/><path d="M7 10l5 5 5-5"/><path d="M4 17v3h16v-3"/>'),
  close: svg('<path d="M6 6l12 12"/><path d="M18 6L6 18"/>'),
} as const;

export type IconName = keyof typeof ICONS;
