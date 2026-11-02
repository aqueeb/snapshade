// Typed coordinate spaces — TypeScript mirror of the Rust core (D-01, D-05).
//
// Every function here mirrors a function in `crates/core/src/geometry.rs` or
// `crates/core/src/topology.rs` 1:1 (same name in camelCase, same f64 maths,
// same `floor(v + 0.5)` rounding). Both implementations are pinned by the one
// shared fixture `fixtures/geometry/displays.json`; neither may edit it.
//
// | Space                  | Types                          | Origin                              |
// |------------------------|--------------------------------|-------------------------------------|
// | Global logical         | LogicalPoint / LogicalRect     | OS virtual desktop, top-left, y-down |
// | Display-local physical | PhysicalPoint / PhysicalRect   | top-left of one display's bitmap     |
//
// The core never sees a logical coordinate at interactive time (ARCHITECTURE
// §Key Data Flows 2): the overlay adapter converts at the edge with `toPhysical`.

/** Stable per-display identifier (CGDirectDisplayID on macOS, adapter index on Windows). */
export type DisplayId = number;

/** A point in global logical space (points on macOS, DIPs on Windows). */
export interface LogicalPoint {
  x: number;
  y: number;
}

/** A rectangle in global logical space; `x`/`y` is the top-left corner. */
export interface LogicalRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A point in display-local physical pixels (top-left of that display's bitmap). */
export interface PhysicalPoint {
  x: number;
  y: number;
}

/** A size in physical pixels. */
export interface PhysicalSize {
  w: number;
  h: number;
}

/** A rectangle in display-local physical pixels; `x`/`y` is the top-left corner. */
export interface PhysicalRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Everything the core needs to know about one display.
 * `physical` must equal `physicalSizeFor(logical, scale)`.
 * @see crates/core/src/geometry.rs `DisplayInfo`
 */
export interface DisplayInfo {
  id: DisplayId;
  logical: LogicalRect;
  scale: number;
  physical: PhysicalSize;
  primary: boolean;
}

/**
 * All connected displays at one instant, primary first then ascending id.
 * @see crates/core/src/topology.rs `DisplayTopology`
 */
export interface DisplayTopology {
  displays: DisplayInfo[];
}

/**
 * The single rounding rule for every logical→physical pixel mapping
 * (RESEARCH Q9 / Pitfall 9): `floor(v + 0.5)`.
 *
 * JavaScript's native round rounds half toward +∞ and Rust's `f64::round` rounds half away
 * from zero; they disagree at -0.5. Both cores use this exact formula.
 * @see crates/core/src/geometry.rs `round_half_up`
 */
export function roundHalfUp(v: number): number {
  return Math.floor(v + 0.5);
}

/** Clamp a rounded pixel coordinate to `[0, extent - 1]` (an empty extent clamps to 0). */
function clampAxis(v: number, extent: number): number {
  const max = Math.max(0, extent - 1);
  return Math.min(Math.max(v, 0), max);
}

/**
 * Global logical point → this display's physical pixel:
 * `roundHalfUp((p - origin) * scale)`, clamped to `[0, w-1] × [0, h-1]`.
 * @see crates/core/src/geometry.rs `DisplayInfo::to_physical`
 */
export function toPhysical(d: DisplayInfo, p: LogicalPoint): PhysicalPoint {
  const x = roundHalfUp((p.x - d.logical.x) * d.scale);
  const y = roundHalfUp((p.y - d.logical.y) * d.scale);
  return { x: clampAxis(x, d.physical.w), y: clampAxis(y, d.physical.h) };
}

/**
 * This display's physical pixel → global logical point (`origin + p / scale`,
 * no rounding).
 * @see crates/core/src/geometry.rs `DisplayInfo::to_logical`
 */
export function toLogical(d: DisplayInfo, p: PhysicalPoint): LogicalPoint {
  return { x: d.logical.x + p.x / d.scale, y: d.logical.y + p.y / d.scale };
}

/**
 * Physical bitmap size of a logical rect at `scale`:
 * `roundHalfUp(w * scale) × roundHalfUp(h * scale)`, never negative.
 * @see crates/core/src/geometry.rs `DisplayInfo::physical_size_for`
 */
export function physicalSizeFor(logical: LogicalRect, scale: number): PhysicalSize {
  return {
    w: Math.max(0, roundHalfUp(logical.w * scale)),
    h: Math.max(0, roundHalfUp(logical.h * scale)),
  };
}

/**
 * Flip an AppKit `NSScreen.frame` (bottom-left origin, y-up, points) into global
 * logical space (top-left origin, y-down) using the height of `NSScreen.screens[0]`
 * — never `NSScreen.main`: `yTop = primaryHeightPt - (y + h)`; `x`, `w`, `h` unchanged.
 * @see crates/core/src/geometry.rs `flip_nsscreen_rect`
 */
export function flipNsScreenRect(
  primaryHeightPt: number,
  x: number,
  y: number,
  w: number,
  h: number,
): LogicalRect {
  return { x, y: primaryHeightPt - (y + h), w, h };
}

/**
 * Build a topology enforcing the ordering invariant: primary first, then
 * ascending id (stable sort; the input array is not mutated).
 * @see crates/core/src/topology.rs `DisplayTopology::new`
 */
export function newTopology(displays: DisplayInfo[]): DisplayTopology {
  const sorted = [...displays].sort((a, b) => {
    if (a.primary !== b.primary) return a.primary ? -1 : 1;
    return a.id - b.id;
  });
  return { displays: sorted };
}

/**
 * The display whose logical rect contains `p` (half-open on each axis:
 * `x ∈ [x0, x0 + w)`); the first match in topology order wins, so a point on a
 * shared edge belongs to exactly one display and a point in a gap to none.
 * @see crates/core/src/topology.rs `DisplayTopology::hit_test`
 */
export function hitTest(t: DisplayTopology, p: LogicalPoint): DisplayId | null {
  for (const d of t.displays) {
    const r = d.logical;
    if (p.x >= r.x && p.x < r.x + r.w && p.y >= r.y && p.y < r.y + r.h) return d.id;
  }
  return null;
}
