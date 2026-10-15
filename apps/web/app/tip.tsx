/**
 * What you are looking at, where you are looking.
 *
 * A readout pinned to the bottom of a chart asks the eye to leave the thing it
 * is pointing at, read a line, and come back — twice per dot. So the label goes
 * to the mark instead, and the readout underneath keeps only the verdict, which
 * is a sentence rather than a name.
 *
 * Drawn in a foreignObject so the plate is real HTML: it shrinks to its own
 * text, wraps nothing, and needs no font metrics guessed at from a character
 * count. The slot around it is a fixed box, and the plate aligns inside that
 * box — centred over the mark, flush to whichever side of it has room, and
 * flipped underneath when the mark is near the top. So it never leaves the
 * chart, at any size, without anything being measured.
 */

const SLOT = 210
const RISE = 54
/** clears the largest mark either chart draws */
const CLEAR = 17

export function Tip({
  at,
  box,
  badge,
  tone,
  on,
  model,
  effort,
  foot,
}: {
  at: { x: number; y: number }
  box: { w: number; h: number }
  badge?: string
  tone?: string
  on?: string
  model: string
  effort?: string
  foot: string
}) {
  const under = at.y - RISE - CLEAR < 0
  const start = at.x < SLOT / 2
  const end = at.x > box.w - SLOT / 2
  return (
    <foreignObject
      className="tip"
      x={start ? at.x - 20 : end ? at.x + 20 - SLOT : at.x - SLOT / 2}
      y={under ? at.y + CLEAR : at.y - CLEAR - RISE}
      width={SLOT}
      height={RISE}
      aria-hidden
    >
      <div className={`tip-slot${start ? " start" : end ? " end" : ""}${under ? " under" : ""}`}>
        <span className="tip-plate">
          <span className="tip-line">
            {badge && (
              <b className="tip-badge" style={{ background: tone, color: on }}>
                {badge}
              </b>
            )}
            <span className="tip-name">
              {model}
              {effort ? <em> {effort}</em> : null}
            </span>
          </span>
          <span className="tip-foot">{foot}</span>
        </span>
      </div>
    </foreignObject>
  )
}
