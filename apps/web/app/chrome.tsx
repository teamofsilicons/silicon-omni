"use client"

import { List, X } from "@phosphor-icons/react/dist/ssr"
import { usePathname } from "next/navigation"
import { useCallback, useEffect, useRef, useState } from "react"

import { Mark } from "./mark"

/** Pages of this site. */
const HERE = [
  { href: "/docs", label: "documentation" },
  { href: "/models", label: "models" },
  { href: "/inspirations", label: "inspirations" },
]

/** Places that are not this site, marked as such. */
const AWAY = [
  { href: "https://github.com/teamofsilicons/silicon-omni", label: "github" },
  { href: "https://pypi.org/project/silicon-omni/", label: "pypi" },
]

/**
 * Two islands, and an indicator that slides.
 *
 * The active page used to be marked by making its label bolder, which reflows
 * the row every time the pointer crosses it — the whole strip twitches. So the
 * state is a background that slides underneath instead: nothing about the type
 * changes except its colour, and colour costs no width.
 */
export function Chrome({ children }: { children: React.ReactNode }) {
  const here = usePathname()
  const [stuck, setStuck] = useState(false)
  const [open, setOpen] = useState(false)
  const strip = useRef<HTMLDivElement>(null)
  const [pill, setPill] = useState<{ x: number; w: number } | null>(null)
  /** the item the pointer is over, if any. The indicator follows it. */
  const [over, setOver] = useState<string | null>(null)
  /** whether there is anything to point at right now */
  const [want, setWant] = useState(false)
  /** one tick behind `want`, so the indicator can fade rather than appear */
  const [shown, setShown] = useState(false)

  useEffect(() => {
    const watch = () => setStuck(window.scrollY > 8)
    watch()
    window.addEventListener("scroll", watch, { passive: true })
    return () => window.removeEventListener("scroll", watch)
  }, [])

  /* One object that moves.
     The indicator parks on the current page, follows the pointer while it is
     over the strip, and goes back when it leaves. With nothing to point at it
     stays where it is and fades — so on a page that is not in the strip it
     fades in under the first item the pointer reaches, and only then slides.
     Never a fly-in from the left edge. */
  const place = useCallback((target?: string | null) => {
    const box = strip.current
    if (!box) return
    const on = target
      ? box.querySelector<HTMLElement>(`a[href="${target}"]`)
      : box.querySelector<HTMLElement>("a[data-here]")
    setWant(!!on)
    if (on) setPill({ x: on.offsetLeft, w: on.offsetWidth })
  }, [])

  useEffect(() => {
    place(over)
    const again = () => place(over)
    window.addEventListener("resize", again)
    const fonts = (document as Document & { fonts?: FontFaceSet }).fonts
    fonts?.ready.then(again)
    return () => window.removeEventListener("resize", again)
  }, [place, here, over])

  /* A frame's grace between mounting the indicator and lighting it, so the
     opacity has somewhere to travel from. */
  useEffect(() => {
    if (!want) return setShown(false)
    const t = setTimeout(() => setShown(true), 40)
    return () => clearTimeout(t)
  }, [want, pill])

  useEffect(() => setOpen(false), [here])

  // Anything marked .enter is released as it arrives, once.
  useEffect(() => {
    const waiting = document.querySelectorAll(".enter:not(.seen)")
    if (!waiting.length) return
    const eye = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            entry.target.classList.add("seen")
            eye.unobserve(entry.target)
          }
        }
      },
      { rootMargin: "0px 0px -12% 0px" },
    )
    waiting.forEach((el) => eye.observe(el))
    return () => eye.disconnect()
  }, [here])

  const mark = (href: string) => (here === href || (href !== "/" && here.startsWith(href)) ? true : false)

  return (
    <>
      <a className="skip" href="#start">
        skip to the page
      </a>

      <header className={`top${stuck ? " stuck" : ""}${open ? " open" : ""}`}>
        <a href="/" className="wordmark" aria-label="silicon omni, home">
          <Mark h={14} />
          <span>silicon omni</span>
        </a>

        <nav className="places" aria-label="pages">
          <div className="strip" ref={strip} onPointerLeave={() => setOver(null)}>
            {pill && (
              <span
                className={`strip-pill${shown ? " shown" : ""}`}
                style={{ transform: `translateX(${pill.x}px)`, width: pill.w }}
                aria-hidden
              />
            )}
            {HERE.map((place) => (
              <a
                key={place.href}
                href={place.href}
                onPointerEnter={() => setOver(place.href)}
                {...(mark(place.href) ? { "data-here": "", "aria-current": "page" as const } : {})}
              >
                {place.label}
              </a>
            ))}
          </div>
          <span className="strip-seam" aria-hidden />
          {AWAY.map((place) => (
            <a key={place.href} className="away" href={place.href} rel="noreferrer">
              {place.label}
            </a>
          ))}
        </nav>

        <button
          className="burger"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-controls="pocket"
          aria-label={open ? "close the menu" : "open the menu"}
        >
          {open ? <X size={16} weight="bold" /> : <List size={16} weight="bold" />}
        </button>
      </header>

      {/* The same list, vertically, for a screen too narrow to line it up. */}
      <div id="pocket" className={`pocket${open ? " open" : ""}`} hidden={!open}>
        {HERE.map((place) => (
          <a
            key={place.href}
            href={place.href}
            {...(mark(place.href) ? { "data-here": "", "aria-current": "page" as const } : {})}
          >
            {place.label}
          </a>
        ))}
        <span className="pocket-seam" aria-hidden />
        {AWAY.map((place) => (
          <a key={place.href} className="away" href={place.href} rel="noreferrer">
            {place.label}
          </a>
        ))}
      </div>

      <main id="start">{children}</main>

      <footer className="end">
        <div className="row">
          <span>silicon omni</span>
          <a href="/docs">documentation</a>
          <a href="/models">models</a>
          <a href="/inspirations">inspirations</a>
          <a href="/choose.json?key=code">the answer, as json</a>
          <a href="/registry.json">what you may ask for</a>
          <a href="https://github.com/teamofsilicons/silicon-omni">library</a>
          <a href="https://pypi.org/project/silicon-omni/">pypi</a>
          <a href="https://github.com/teamofsilicons/silicon-omni/blob/main/CHANGELOG.md">changelog</a>
          <a href="https://github.com/teamofsilicons/silicon-omni/tree/main/apps/web">this site</a>
          <a href="https://artificialanalysis.ai/evaluations/gdpval-aa">gdpval</a>
        </div>
      </footer>
    </>
  )
}
