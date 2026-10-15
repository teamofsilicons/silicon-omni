"use client"

import { Check, Copy as CopyIcon } from "@phosphor-icons/react/dist/ssr"
import { useState } from "react"

/**
 * A code block that says what language it is, and colours accordingly.
 *
 * The highlighter is a regex per language. It finds comments, strings,
 * keywords, numbers, decorators and the names being called, and leaves
 * everything else as ink — so anything it fails to recognise stays black,
 * which is the correct failure.
 *
 * The four colours are the ramp's own stops, each dragged toward the ink until
 * it clears 4.5:1 on this ground. Blue is absent on purpose: on this site blue
 * means a thing you can click.
 */

type Piece = { text: string; kind?: string }

const KEYWORDS =
  "from|import|as|def|class|return|if|elif|else|for|while|in|is|and|or|not|None|True|False|" +
  "lambda|try|except|finally|raise|with|yield|pass|break|continue|global|nonlocal|assert|del|await|async"

const COMMENT = `(?<comment>#[^\\n]*)`
const STRING =
  `(?<string>"""[\\s\\S]*?"""|'''[\\s\\S]*?'''|"(?:\\\\.|[^"\\\\])*"|'(?:\\\\.|[^'\\\\])*')`
const RUST_KEYWORDS =
  "as|async|await|break|const|continue|crate|dyn|else|enum|extern|false|fn|for|if|impl|in|let|loop|match|mod|move|mut|pub|ref|return|self|Self|static|struct|super|trait|true|type|unsafe|use|where|while"
const RUST_COMMENT = `(?<comment>//[^\\n]*|/\\*[\\s\\S]*?\\*/)`
const RUST_STRING = `(?<string>"(?:\\\\.|[^"\\\\])*")`

const GRAMMAR: Record<string, RegExp> = {
  python: new RegExp(
    [
      COMMENT,
      STRING,
      `(?<decorator>@[A-Za-z_][\\w.]*)`,
      `(?<keyword>\\b(?:${KEYWORDS})\\b)`,
      `(?<number>\\b\\d+(?:\\.\\d+)?\\b)`,
      `(?<call>\\b[A-Za-z_]\\w*(?=\\())`,
    ].join("|"),
    "g",
  ),
  bash: new RegExp([COMMENT, STRING, `(?<call>^[a-z][\\w.-]*)`].join("|"), "gm"),
  http: new RegExp(
    [`(?<keyword>^(?:GET|POST|PUT|PATCH|DELETE)\\b)`, `(?<number>\\b\\d+\\b)`].join("|"),
    "gm",
  ),
  rust: new RegExp(
    [
      RUST_COMMENT,
      RUST_STRING,
      `(?<decorator>#!?\\[[^\\]]+\\])`,
      `(?<keyword>\\b(?:${RUST_KEYWORDS})\\b)`,
      `(?<number>\\b\\d+(?:\\.\\d+)?\\b)`,
      `(?<call>\\b[A-Za-z_]\\w*(?=\\s*[!(]))`,
    ].join("|"),
    "g",
  ),
}

/** Source split into coloured runs. Plain text comes back with no kind. */
export function pieces(source: string, lang = "python"): Piece[] {
  const scan = GRAMMAR[lang]
  if (!scan) return [{ text: source }]
  const out: Piece[] = []
  let at = 0
  for (const m of source.matchAll(scan)) {
    const i = m.index ?? 0
    if (i > at) out.push({ text: source.slice(at, i) })
    const kind = Object.entries(m.groups ?? {}).find(([, v]) => v !== undefined)?.[0]
    out.push({ text: m[0], kind })
    at = i + m[0].length
  }
  if (at < source.length) out.push({ text: source.slice(at) })
  return out
}

/** The runs, as spans. Use this where a block's chrome would be too much. */
export function Tokens({ children, lang = "python" }: { children: string; lang?: string }) {
  return (
    <>
      {pieces(children, lang).map((piece, i) =>
        piece.kind ? (
          <span key={i} className={`t-${piece.kind}`}>
            {piece.text}
          </span>
        ) : (
          piece.text
        ),
      )}
    </>
  )
}

export function Code({ children, lang = "python" }: { children: string; lang?: string }) {
  const [done, setDone] = useState(false)
  return (
    <figure className="code">
      <figcaption className="code-bar">
        <span className="code-lang num">{lang}</span>
        <button
          className={`code-take num${done ? " done" : ""}`}
          onClick={() => {
            navigator.clipboard.writeText(children).then(() => {
              setDone(true)
              setTimeout(() => setDone(false), 1600)
            })
          }}
          aria-label={done ? "copied" : "copy this code"}
        >
          {done ? <Check size={12} weight="bold" /> : <CopyIcon size={12} weight="bold" />}
          {done ? "copied" : "copy"}
        </button>
      </figcaption>
      <pre>
        <code>
          <Tokens lang={lang}>{children}</Tokens>
        </code>
      </pre>
    </figure>
  )
}
