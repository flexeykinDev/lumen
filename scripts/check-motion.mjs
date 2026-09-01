// Asserts that src/lib/motion.ts agrees with src-tauri/src/motion.rs.
//
// The host animates the window region and the renderer animates the panel; they
// share no clock, only this curve. If the two implementations drift, the glass
// edge separates from its contents mid-transition — a bug that is obvious on
// screen and invisible in every other test.
//
// The .ts module is imported directly: Node strips the types itself, so this
// tests the exact source the bundler ships rather than a transformed copy.

import { EASE_CSS, ease } from "../src/lib/motion.ts";

// cubic-bezier(0.22, 1, 0.36, 1) solved by 200-step bisection — an independent
// method from the Newton solver under test. Identical table to the Rust test in
// src-tauri/src/motion.rs.
const EXPECTED = [
  0.0, 0.401096896, 0.673978074, 0.832192057, 0.917146858, 0.961382548, 0.983675387,
  0.994193844, 0.998524422, 0.999839526, 1.0,
];

let failures = 0;

for (let i = 0; i <= 10; i++) {
  const x = i / 10;
  const got = ease(x);
  if (Math.abs(got - EXPECTED[i]) > 1e-4) {
    console.error(`  ease(${x}) = ${got}, expected ${EXPECTED[i]}`);
    failures++;
  }
}

// The CSS string and the numeric solver must describe the same curve, or the
// stylesheet transitions and the JS-driven ones diverge from each other too.
if (EASE_CSS !== "cubic-bezier(0.22, 1, 0.36, 1)") {
  console.error(`  EASE_CSS = ${EASE_CSS}, expected cubic-bezier(0.22, 1, 0.36, 1)`);
  failures++;
}

// Monotonicity: a non-monotonic easing makes the capsule visibly jitter.
let previous = -1;
for (let i = 0; i <= 200; i++) {
  const v = ease(i / 200);
  if (v < previous - 1e-9 || v < 0 || v > 1) {
    console.error(`  ease(${i / 200}) = ${v} broke monotonicity or range`);
    failures++;
    break;
  }
  previous = v;
}

if (failures > 0) {
  console.error(`\nmotion.ts has drifted from motion.rs (${failures} failures).`);
  process.exit(1);
}

console.log("motion.ts matches motion.rs across 11 sample points and 200 monotonicity steps.");
