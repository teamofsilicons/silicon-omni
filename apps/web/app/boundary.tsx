/**
 * The turn boundary.
 *
 * Two things arrive while a turn is running, and omni treats them as opposites
 * on purpose. A message is injected — it lands inside the turn, because that is
 * what you meant by sending it now. A change of model is held — it lands at the
 * seam, because a model changing underneath itself halfway through a tool call
 * is not a feature, it is a bug with a marketing name.
 *
 * Both are recorded the moment you ask. Only one is applied then.
 */

export function Boundary() {
  return (
    <div className="bound">
      <div className="bound-row">
        <span className="bound-tag num">turn 4</span>
        <div className="bound-track">
          <div className="bound-bar">
            <span className="bound-seg" style={{ width: "20%" }}>thinking</span>
            <span className="bound-seg tool" style={{ width: "40%" }}>Bash: pytest</span>
            <span className="bound-seg" style={{ width: "40%" }}>text</span>
          </div>
          <span className="bound-seam" aria-hidden />
          <span className="bound-seam-tag num">boundary</span>
        </div>
      </div>

      <div className="bound-row">
        <span className="bound-tag num">arrives</span>
        <div className="bound-track">
          <span className="bound-mark" style={{ left: "46%" }} aria-hidden>
            <i />
          </span>
          <p className="bound-say">
            <code>chat.send(&quot;also check the fixtures&quot;)</code> <b>injected.</b> It lands
            where you sent it, inside the turn already running. <code>send</code> returns after the
            daemon has durably accepted it, without waiting for model output, and is safe from any
            thread, including from inside an event handler.
          </p>
        </div>
      </div>

      <div className="bound-row">
        <span className="bound-tag num">arrives</span>
        <div className="bound-track">
          <span className="bound-mark held" style={{ left: "46%" }} aria-hidden>
            <i />
            <s />
          </span>
          <p className="bound-say">
            <code>chat.intelligence(9)</code> <b>held.</b> Written down the moment you ask, applied
            at the seam once the running tool has finished. Providers, prompts and session swaps
            wait in the same queue.
          </p>
        </div>
      </div>
    </div>
  )
}
