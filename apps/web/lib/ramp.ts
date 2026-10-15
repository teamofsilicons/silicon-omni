/**
 * The ramp, sampled.
 *
 * The site is given one gradient and told to use each colour alone when it
 * stands for one thing, and the whole gradient when it shows things working
 * together. The dial is the second case: eleven levels made out of three
 * vendors' models, and no single vendor owns it. So the dial is the gradient,
 * sampled eleven times — level 0 at the shallow end, level 10 in the deep.
 *
 * The SVG marks come from here. The `--dial-*` variables in globals.css are
 * printed by the snippet at the bottom of this file and pasted in, which is a
 * copy and therefore a thing that can drift — it has, twice, by one hex digit.
 * Treat this file as the source and re-paste rather than editing the CSS.
 */

/** The gradient as given, as stop positions down a 180deg axis. */
export const RAMP: [number, string][] = [
  [0.0, "#76D4F0"],
  [0.2, "#F9F987"],
  [0.6, "#FEAD75"],
  [0.8, "#F15347"],
  [1.0, "#14245F"],
]

const hex = (s: string) => [
  parseInt(s.slice(1, 3), 16),
  parseInt(s.slice(3, 5), 16),
  parseInt(s.slice(5, 7), 16),
]

const pad = (n: number) => Math.round(Math.min(255, Math.max(0, n))).toString(16).padStart(2, "0")

/** The colour of the gradient at `t`, 0 at the top stop and 1 at the bottom. */
export function sample(t: number): string {
  const at = Math.min(1, Math.max(0, t))
  let i = 0
  while (i < RAMP.length - 2 && at > RAMP[i + 1][0]) i++
  const [t0, c0] = RAMP[i]
  const [t1, c1] = RAMP[i + 1]
  const f = t1 === t0 ? 0 : (at - t0) / (t1 - t0)
  const a = hex(c0)
  const b = hex(c1)
  return `#${pad(a[0] + (b[0] - a[0]) * f)}${pad(a[1] + (b[1] - a[1]) * f)}${pad(a[2] + (b[2] - a[2]) * f)}`
}

export const LEVELS = 11

/** Level 0 is the shallow end of the ramp; level 10 is the deep. */
export function dial(level: number): string {
  return sample(level / (LEVELS - 1))
}

/* There used to be a `legible()` here that mixed a ramp stop toward the ink so
   it could be set as text, and a docstring claiming a colour could therefore
   never be one value in the stylesheet and another in the chart. Nothing
   called it, and the three --*-ink variables in globals.css had been hand-tuned
   away from what it returned — so the only thing it actually did was make a
   promise. The vendor inks are declared once, in CSS, and measured there. */

/** Relative luminance, used to decide whether a level's numeral goes light or dark. */
export function luminance(colour: string): number {
  const [r, g, b] = hex(colour).map((v) => {
    const s = v / 255
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/**
 * Ink or paper, whichever can actually be read on top of `colour`.
 *
 * The threshold is 0.2 and not the tempting 0.42, because measured against the
 * eleven dial stops the higher one hands paper to level 7 (2.51:1) and level 8
 * (3.41:1) — both illegible, both fixed by ink at 6.71:1 and 4.93:1. Anything
 * darker than about a fifth of full luminance is safer with paper on it; the
 * whole warm middle of this ramp is not.
 */
export function on(colour: string): string {
  return luminance(colour) > 0.2 ? "#1A1A1A" : "#FFFDF9"
}
