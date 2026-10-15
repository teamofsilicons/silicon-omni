"use client"

import {
  Broadcast,
  ChatCircleDots,
  Database,
  TestTube,
  Door,
  DownloadSimple,
  Files,
  Gauge,
  Key,
  TerminalWindow,
  Timer,
  Wrench,
  Eye,
} from "@phosphor-icons/react/dist/ssr"
import { useEffect, useState } from "react"

import { Dither } from "../dither"
import { Mark } from "../mark"
import { sample } from "../../lib/ramp"
import { Ticks } from "../ticks"
import { SECTIONS } from "./content"
import { Code } from "../code"

/**
 * The reference, with an optional second layer.
 *
 * "internal" is not a separate page, because the two would drift. It is the
 * same list with a second paragraph revealed, so what a call does and what it
 * turns into stay side by side.
 */

const ICONS: Record<string, React.ComponentType<{ size?: number; weight?: "bold" | "duotone" }>> = {
  DownloadSimple,
  TerminalWindow,
  Door,
  ChatCircleDots,
  Broadcast,
  Files,
  Key,
  Gauge,
  Database,
  TestTube,
  Timer,
}

export function Docs() {
  const [inside, setInside] = useState(false)
  const [here, setHere] = useState(SECTIONS[0].id)

  useEffect(() => {
    const eye = new IntersectionObserver(
      (entries) => {
        const showing = entries.filter((e) => e.isIntersecting)
        if (showing.length) setHere(showing[0].target.id)
      },
      { rootMargin: "-88px 0px -62% 0px" },
    )
    SECTIONS.forEach((s) => {
      const el = document.getElementById(s.id)
      if (el) eye.observe(el)
    })
    return () => eye.disconnect()
  }, [])

  return (
    <div className="shell docs">
      <Dither shape="linear" cells={200} />
      <aside className="toc">
        <ol>
          {SECTIONS.map((s) => (
            <li key={s.id} style={{ ["--tone" as string]: sample(SECTIONS.indexOf(s) / (SECTIONS.length - 1)) }}>
              <a href={`#${s.id}`} className={here === s.id ? "here" : undefined}>
                {(() => {
                  const Icon = ICONS[s.icon]
                  return Icon ? <Icon size={15} weight="bold" /> : null
                })()}
                {s.title}
              </a>
            </li>
          ))}
        </ol>
        <div className="toc-switch">
          {/* Two buttons, not a tablist. The role promised arrow keys, aria-selected
              and tabpanels, none of which were here. aria-pressed is the truth. */}
          <div className="switch">
            <button
              className={inside ? undefined : "on"}
              onClick={() => setInside(false)}
              aria-pressed={!inside}
            >
              <Eye size={13} weight="bold" />
              surface
            </button>
            <button
              className={inside ? "on" : undefined}
              onClick={() => setInside(true)}
              aria-pressed={inside}
            >
              <Wrench size={13} weight="bold" />
              internal
            </button>
          </div>
          <p className="toc-hint">
            {inside
              ? "showing what each call turns into underneath"
              : "what each call does, and what it takes"}
          </p>
        </div>
      </aside>

      <div className="docs-body">
        <h1 className="rise">
          Documentation.{" "}
          <a
            className="release num"
            href="https://pypi.org/project/silicon-omni/"
            title="the release this page documents"
          >
            v0.7.0
          </a>
        </h1>
        <p className="lede rise" style={{ ["--i" as string]: 1, marginTop: "1.3rem", maxWidth: "34rem" }}>
          Every call omni exposes. Flip to <strong>internal</strong> at any point to see the flag,
          the JSON-RPC method or the file write it actually becomes.
        </p>

        {SECTIONS.map((section, i) => (
          <section
            key={section.id}
            id={section.id}
            className="doc-section enter"
            style={{ ["--tone" as string]: sample(i / (SECTIONS.length - 1)) }}
          >
            <div className="rule">
              <i className="rule-swatch" />
              {(() => {
                const Icon = ICONS[section.icon]
                return Icon ? <Icon size={14} weight="bold" /> : null
              })()}
              <h2 className="rule-title">{section.title.toLowerCase()}</h2>
            </div>
            <p className="doc-blurb"><Ticks>{section.blurb}</Ticks></p>

            {section.code && (
              <Code lang={section.lang}>{section.code}</Code>
            )}

            {section.entries && (
              <dl className="entries">
                {section.entries.map((entry) => (
                  <div key={entry.call} className="entry">
                    <dt>
                      <code>{entry.call}</code>
                      {entry.takes && <span className="tag">{entry.takes}</span>}
                      {entry.gives && <span className="tag">→ {entry.gives}</span>}
                    </dt>
                    <dd>
                      <p><Ticks>{entry.does}</Ticks></p>
                      {entry.inside && (
                        <details className="inside" open={inside}>
                          <summary>underneath</summary>
                          <p>
                            <Ticks>{entry.inside}</Ticks>
                          </p>
                        </details>
                      )}
                    </dd>
                  </div>
                ))}
              </dl>
            )}

            {section.note && (
              <p className="doc-note">
                <Ticks>{section.note}</Ticks>
              </p>
            )}
          </section>
        ))}

        <div className="breath">
          <Mark h={16} />
        </div>
      </div>
    </div>
  )
}
