const STOPS = [
  { at: "Omni admin", what: "sign in, edit the catalog, publish changes" },
  { at: "versioned catalog", what: "validated changes, with earlier versions retained" },
  { at: "/choose.json", what: "a word, a number, or a model you name" },
  { at: "~/.omni/cache", what: "kept an hour. a stale answer still runs offline" },
  { at: "claude -p --model … --effort max", what: "two strings, handed over as they are" },
]

export function Pipeline() {
  return (
    <ol className="pipe">
      {STOPS.map((stop) => (
        <li key={stop.at} className="pipe-stop">
          <span className="pipe-dot" aria-hidden />
          <code className="pipe-at">{stop.at}</code>
          <span className="pipe-what num">{stop.what}</span>
        </li>
      ))}
    </ol>
  )
}
