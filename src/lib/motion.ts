// MUST stay numerically identical to src-tauri/src/motion.rs.
// The host animates the window region and the renderer animates the content;
// they never share a clock, only this curve. See ARCHITECTURE.md §4.

/** The one easing curve in the app. easeOutQuint-like: fast out, long settle. */
export const EASE = { x1: 0.22, y1: 1.0, x2: 0.36, y2: 1.0 } as const;

/** CSS form of `EASE`, for transitions declared in stylesheets. */
export const EASE_CSS = `cubic-bezier(${EASE.x1}, ${EASE.y1}, ${EASE.x2}, ${EASE.y2})`;

export const DURATION = {
  expand: 340,
  collapse: 280,
  reveal: 260,
  conceal: 220,
} as const;

function bezierAxis(t: number, a1: number, a2: number): number {
  const u = 1 - t;
  return 3 * u * u * t * a1 + 3 * u * t * t * a2 + t * t * t;
}

/**
 * Evaluate the CSS cubic-bezier at `x` (0..1) by Newton refinement over the
 * parametric form, falling back to bisection when the derivative collapses.
 * ~1e-6 accurate in <=8 iterations for this curve.
 */
export function ease(x: number): number {
  if (x <= 0) return 0;
  if (x >= 1) return 1;

  let t = x;
  for (let i = 0; i < 8; i++) {
    const err = bezierAxis(t, EASE.x1, EASE.x2) - x;
    if (Math.abs(err) < 1e-6) return bezierAxis(t, EASE.y1, EASE.y2);
    const u = 1 - t;
    const d =
      3 * u * u * EASE.x1 + 6 * u * t * (EASE.x2 - EASE.x1) + 3 * t * t * (1 - EASE.x2);
    if (Math.abs(d) < 1e-6) break;
    t -= err / d;
  }

  let lo = 0;
  let hi = 1;
  t = x;
  for (let i = 0; i < 24; i++) {
    const v = bezierAxis(t, EASE.x1, EASE.x2);
    if (Math.abs(v - x) < 1e-6) break;
    if (v > x) hi = t;
    else lo = t;
    t = (lo + hi) / 2;
  }
  return bezierAxis(t, EASE.y1, EASE.y2);
}

export const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

export const clamp01 = (v: number): number => (v < 0 ? 0 : v > 1 ? 1 : v);
