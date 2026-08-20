"use client"

import { useState } from "react"

export function Copy({ text }: { text: string }) {
  const [done, setDone] = useState(false)
  return (
    <button
      className="copy-btn"
      onClick={() => {
        navigator.clipboard.writeText(text).then(() => {
          setDone(true)
          setTimeout(() => setDone(false), 1600)
        })
      }}
    >
      {done ? "copied" : "copy"}
    </button>
  )
}
