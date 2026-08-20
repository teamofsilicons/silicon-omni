import type { Metadata } from "next"
import { Geist_Mono, Inter } from "next/font/google"

import "./globals.css"

const inter = Inter({ subsets: ["latin"], variable: "--font-sans", display: "swap" })
const mono = Geist_Mono({ subsets: ["latin"], variable: "--font-mono", display: "swap" })

export const metadata: Metadata = {
  title: "silicon omni | One Interface for Claude Code, Codex and Antigravity",
  description:
    "One Python interface for Claude Code, Codex and Antigravity, driven by the subscriptions you already pay for. Conversations move between providers mid-flight and carry on where they left off.",
  openGraph: {
    title: "silicon omni | One Interface for Claude Code, Codex and Antigravity",
    description:
      "One Python interface for three agentic CLIs. Move a conversation between vendors mid-flight and it carries on where it left off.",
    siteName: "silicon omni",
    type: "website",
  },
}

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className={`${inter.variable} ${mono.variable}`}>
        <div className="site">{children}</div>
      </body>
    </html>
  )
}
