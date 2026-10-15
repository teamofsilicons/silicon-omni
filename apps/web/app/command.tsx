"use client"

import { Check, Copy as CopyIcon } from "@phosphor-icons/react/dist/ssr"
import { useState } from "react"

/**
 * The command, and copying it, as one thing.
 *
 * There was a dark block with the command in it and a separate pale button
 * beside it saying "copy" — two controls for one intention, and the second one
 * had to be read to be understood. Now the block is the button: the icon lives
 * inside it at the right, behind a hairline, and clicking anywhere on the
 * command copies the command.
 */
export function Command({ text }: { text: string }) {
  const [done, setDone] = useState(false)
  return (
    <button
      className={`command${done ? " done" : ""}`}
      onClick={() => {
        navigator.clipboard.writeText(text).then(() => {
          setDone(true)
          setTimeout(() => setDone(false), 1600)
        })
      }}
      aria-label={done ? `copied: ${text}` : `copy to clipboard: ${text}`}
    >
      <code>{text}</code>
      <span className="command-take" aria-hidden>
        {done ? <Check size={13} weight="bold" /> : <CopyIcon size={13} weight="bold" />}
      </span>
    </button>
  )
}
