import type { Metadata } from "next"

import { Chrome } from "./chrome"
import "./globals.css"

export const metadata: Metadata = {
  title: "silicon omni — one session across Claude Code, Codex and Antigravity",
  description:
    "The conversation is yours and the vendor is a setting. Python, Rust and terminal clients share one daemon that drives three agentic CLIs, keeps the log in a file it owns, and takes an integer from 0 to 10 instead of a model name.",
  openGraph: {
    title: "silicon omni",
    description:
      "One persistent session across Claude Code, Codex and Antigravity. Move a conversation between vendors mid-run and it carries on with everything said before it already in its head.",
    siteName: "silicon omni",
    type: "website",
  },
}

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        {/* Everything marked .enter waits for an observer to release it. With no
            script there is no observer, so without this the page below the hero
            is present, correct, and invisible. */}
        <noscript>
          <style>{`.enter { opacity: 1 !important; transform: none !important; }`}</style>
        </noscript>
      </head>
      <body>
        <Chrome>{children}</Chrome>
      </body>
    </html>
  )
}
