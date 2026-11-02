// Overlay colour / size tokens (01-UI-REVIEW fix 3; D-19 `src/theme/`). Single source for
// every literal the overlay paints so the pre-dim, the drag repaint and the selection chrome
// can never drift apart.

/** 50 % black veil over the frozen frame (spec §3.2 "dimmed/greyed out"). */
export const DIM_FILL = "rgba(0,0,0,0.5)";
/** 1-px selection border. */
export const SELECTION_BORDER = "#ffffff";
/** Resize handle fill / stroke (8 squares). */
export const HANDLE_FILL = "#ffffff";
export const HANDLE_STROKE = "#000000";
/** Handle square half-size in CSS px (hit slop in the model is in physical px). */
export const HANDLE_HALF_PT = 3;
/** Size badge: `W×H` in Arial. */
export const BADGE_BG = "rgba(0,0,0,0.7)";
export const BADGE_FG = "#ffffff";
export const BADGE_FONT = "12px Arial, Helvetica, sans-serif";
/** Gap between the selection and a toolbar / badge in CSS px. */
export const TOOLBAR_GAP_PT = 4;
/** Default colours (spec §4: Color default red, Marker default yellow). */
export const DEFAULT_COLOR = "#ff0000";
export const DEFAULT_MARKER_COLOR = "#ffff00";
/**
 * Swatch palette for the Color button (spec §4 "palette swatches + custom color dialog" —
 * the Windows picker; D-25). 16 classic colours, red first.
 */
export const PALETTE: readonly string[] = [
  "#ff0000", "#ff8000", "#ffff00", "#00ff00", "#00ffff", "#0000ff", "#8000ff", "#ff00ff",
  "#ffffff", "#c0c0c0", "#808080", "#404040", "#000000", "#800000", "#008000", "#000080",
];
