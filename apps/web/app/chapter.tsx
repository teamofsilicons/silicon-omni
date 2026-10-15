/**
 * A chapter, and a plate.
 *
 * Chapters carry a folio in the margin rather than a label above the heading —
 * a running head, the way a long argument is numbered in a printed book. Each
 * one is allowed exactly one colour, sampled from the same gradient the dial
 * is sampled from, so reading the swatches from the top of the page to the
 * bottom is reading the gradient.
 *
 * A plate's credit is not written here. It is read out of plates.json, which is
 * also the licence audit and the input to the printing script — so a caption
 * cannot drift from the file that says where the picture came from. The only
 * part written by hand is the last sentence, which is the only part that is an
 * argument rather than a fact.
 */

import book from "../plates.json"
import { sample } from "../lib/ramp"

type Spec = {
  title: string
  date: string
  holder: string
  licence: string
  source: string
  size?: number[]
}

const PLATES = book.plates as Record<string, Spec>

export function tone(index: number, of: number): string {
  return sample(of <= 1 ? 0 : index / (of - 1))
}

export function Chapter({
  n,
  of,
  name,
  id,
  title,
  argue,
  aside,
  children,
}: {
  n: number
  of: number
  name: string
  id?: string
  title: React.ReactNode
  argue: React.ReactNode
  /** goes under the claim, in the left column, so the column is not a hole */
  aside?: React.ReactNode
  children?: React.ReactNode
}) {
  return (
    <section className="ch enter" id={id} style={{ ["--tone" as string]: tone(n - 1, of) }}>
      <div className="ch-folio">
        <b>{String(n).padStart(2, "0")}</b>
        <i />
        <u>{name}</u>
        <span>of {of}</span>
      </div>
      <div className="ch-body">
        <div className="ch-lead">
          <div className="ch-claim">
            <h2>{title}</h2>
            {aside}
          </div>
          <div className="set">{argue}</div>
        </div>
        {children}
      </div>
    </section>
  )
}

export function Plate({
  name,
  why,
  alt,
  aside,
  eager,
}: {
  name: string
  why: React.ReactNode
  alt?: string
  aside?: boolean
  /** the one plate above the fold: lazy-loading it only delays the LCP */
  eager?: boolean
}) {
  const spec = PLATES[name]
  if (!spec) return null
  const [w, h] = spec.size ?? [720, 460]
  return (
    <figure className={`plate${aside ? " aside" : ""}`}>
      <div className="plate-mount">
        <img
          src={`/plates/${name}.png`}
          srcSet={`/plates/${name}.png 1x, /plates/${name}@2x.png 2x`}
          alt={alt ?? spec.title}
          width={w}
          height={h}
          loading={eager ? "eager" : "lazy"}
          decoding="async"
          fetchPriority={eager ? "high" : undefined}
        />
      </div>
      <figcaption>
        <b>{spec.title}</b>, <span className="plate-when">{spec.date}</span>.{" "}
        <a href={spec.source} rel="noreferrer">
          {spec.holder}
        </a>
        . <em>{why}</em>
      </figcaption>
    </figure>
  )
}
