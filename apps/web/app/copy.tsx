"use client"

import { Check, Copy as CopyIcon } from "@phosphor-icons/react/dist/ssr"
import { useState } from "react"

export function Copy({ text }: { text: string }) {
  const [done, setDone] = useState(false)
  return (
    <button
      className={`copy${done ? " done" : ""}`}
      onClick={() => {
        navigator.clipboard.writeText(text).then(() => {
          setDone(true)
          setTimeout(() => setDone(false), 1500)
        })
      }}
      aria-label={done ? "copied" : "copy to clipboard"}
    >
      {done ? <Check size={13} weight="bold" /> : <CopyIcon size={13} weight="bold" />}
      {done ? "copied" : "copy"}
    </button>
  )
}
