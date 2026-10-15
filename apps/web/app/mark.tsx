/**
 * The mark. Four circles of one size, overlapping.
 *
 * Forty across, twenty-seven apart, so each one sits thirteen inside the last.
 * Four separate things and one shape, which is the entire product and a good
 * deal shorter than this website's attempt to say it.
 *
 * It has two states and they follow the rule the rest of the page follows. In
 * ink it is one thing, and that is the wordmark. In colour it is four stops of
 * the one gradient set to overprint — so the overlaps darken the way two spot
 * colours darken on a press — and that is the mark used only where the point
 * being made is that separate things are cooperating.
 */

import { sample } from "../lib/ramp"

const SPACING = 27
const R = 20
const FIRST = 60

const CIRCLES = [0, 1, 2, 3].map((i) => FIRST + i * SPACING)
const LEFT = FIRST - R
const RIGHT = CIRCLES[3] + R

export function Mark({
  h = 13,
  ramp = false,
  className,
}: {
  h?: number
  ramp?: boolean
  className?: string
}) {
  const w = (h * (RIGHT - LEFT)) / (R * 2)
  return (
    <svg
      className={`mark${ramp ? " over" : ""}${className ? ` ${className}` : ""}`}
      width={w}
      height={h}
      viewBox={`${LEFT} ${100 - R} ${RIGHT - LEFT} ${R * 2}`}
      aria-hidden
      focusable="false"
    >
      {CIRCLES.map((cx, i) => (
        <circle
          key={cx}
          cx={cx}
          cy={100}
          r={R}
          fill={ramp ? sample(i / (CIRCLES.length - 1)) : "currentColor"}
        />
      ))}
    </svg>
  )
}
