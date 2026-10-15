import book from "../../plates.json"
import { Mark } from "../mark"
import { Plate } from "../chapter"
import { FACETS } from "./content"
import { Index } from "./index-list"
import { sample } from "../../lib/ramp"

const DEBTS = FACETS.reduce((n, f) => n + f.notes.length, 0)

export const metadata = {
  title: "inspirations — silicon omni",
  description:
    "Where this site came from. Debts across ten kinds of design: how a thing looks, feels, is understood, is shared, is experienced, what it conveys, how it reads, how it is laid out, how it moves, how it answers to a hand.",
}

type Spec = { title: string; date: string; holder: string; licence: string; source: string }
const PLATES = Object.entries(book.plates as Record<string, Spec>)

/* Two plates ride inside the list rather than at the top of it, so the page
   reads as an argument with illustrations and not a gallery with captions. */
const ILLUSTRATE: Record<string, { name: string; alt: string; why: React.ReactNode }> = {
  looks: {
    name: "diatoms",
    alt: "Plate 41 of Haeckel's Radiolaria: eleven numbered skeletons on bare white",
    why: (
      <>
        Radiolaria, plate 41 of the Challenger report, engraved by Adolf Giltsch. Eleven specimens,
        numbered, on bare white, with the names set underneath. An atlas plate is a scatter plot
        that has been told to sit still.
      </>
    ),
  },
  shared: {
    name: "pigeon-post",
    alt: "A Signal Corps soldier having a message capsule clipped to a carrier pigeon's leg, c. 1917",
    why: (
      <>
        Two places that cannot speak to each other, and a third thing carrying the sentence between
        them. The bird knows nothing about either army.
      </>
    ),
  },
  experienced: {
    name: "murmuration",
    alt: "A dense band of red-winged blackbirds along the horizon, its top edge rolling",
    why: (
      <>
        Nobody drew the top edge of that flock, and it is the sharpest line in the photograph. Every
        frontier on this site is made the same way.
      </>
    ),
  },
}

export default function Inspirations() {
  return (
    <div className="shell rails">
      <section className="ins-head">
        <Mark h={26} ramp className="ins-mark" />
        <h1 className="rise">Inspirations.</h1>
        <p className="lede rise set" style={{ ["--i" as string]: 1, marginTop: "1.3rem", maxWidth: "40rem" }}>
          Design is not only how a thing looks. It is how it feels, how it is understood, how it
          travels, how it is used over time, what idea it carries, how it reads, how it is laid out,
          how it moves, and how it answers to a hand. Ten headings, then, and {DEBTS} debts, and under
          each one the mechanic we actually took rather than the compliment we owe.
        </p>
        <p className="rise quiet" style={{ ["--i" as string]: 2, marginTop: "1.1rem", maxWidth: "36rem", fontSize: 14 }}>
          The second line of every entry is ours. If a connection seems far-fetched, it is, and it
          was still the reason. The mark above is the only one on this site printed in colour: four
          stops of the one gradient, set to overprint, because up there it is not standing for a
          product but for the thing this page is about.
        </p>
      </section>

      <Index facets={FACETS.map((f) => ({ id: f.id, title: f.short }))} />

      {FACETS.map((facet, i) => {
        const tone = sample(i / (FACETS.length - 1))
        const art = ILLUSTRATE[facet.id]
        return (
          <section
            key={facet.id}
            id={facet.id}
            className="ch enter facet"
            style={{ ["--tone" as string]: tone }}
          >
            <div className="ch-folio">
              <b>{String(i + 1).padStart(2, "0")}</b>
              <i />
              <u>{facet.short}</u>
              <span>of {FACETS.length}</span>
            </div>
            <div className="ch-body">
              <div className="ch-lead">
                <h2>{facet.title}</h2>
                <div className="set">
                  <p>{facet.blurb}</p>
                </div>
              </div>

              {art && <Plate name={art.name} alt={art.alt} why={art.why} />}

              <ol className="notes-grid">
                {facet.notes.map((note) => (
                  <li key={note.name} className="note">
                    <a className="note-name" href={note.url} rel="noreferrer">
                      {note.name}
                    </a>
                    <span className="note-who num">
                      {note.maker} · {note.year}
                    </span>
                    <p className="note-mech set">{note.mechanic}</p>
                    <p className="note-took set">{note.taken}</p>
                  </li>
                ))}
              </ol>
            </div>
          </section>
        )
      })}

      <section className="ch enter" id="pictures" style={{ ["--tone" as string]: "var(--ink-34)" }}>
        <div className="ch-folio">
          <b>{PLATES.length}</b>
          <i />
          <u>pictures</u>
          <span>and their papers</span>
        </div>
        <div className="ch-body">
          <div className="ch-lead">
            <h2>Every picture here, and where it came from.</h2>
            <div className="set">
              <p>
                Public domain, PD-US, CC0 or no-known-restrictions, and nothing else. Several better
                photographs were turned down for being share-alike, which is why there is no Chand
                Baori on this site and no starling murmuration.
              </p>
              <p>
                All of them are printed to one bit through the same ordered Bayer matrix. That is
                what makes a wood engraving from 1846 and a photograph from 2015 look like plates
                out of the same book.
              </p>
            </div>
          </div>

          <table className="rows credits">
            <thead>
              <tr>
                <th>plate</th>
                <th>what it is</th>
                <th style={{ width: "12rem" }}>held by</th>
                <th style={{ width: "9rem" }}>licence</th>
              </tr>
            </thead>
            <tbody>
              {PLATES.map(([name, spec]) => (
                <tr key={name}>
                  <td className="n">{name}</td>
                  <td>
                    <a className="link" href={spec.source} rel="noreferrer">
                      {spec.title}
                    </a>
                    <span className="quiet">, {spec.date}</span>
                  </td>
                  <td className="quiet">{spec.holder}</td>
                  <td className="n">{spec.licence}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <div className="breath"><Mark h={16} /></div>
    </div>
  )
}
