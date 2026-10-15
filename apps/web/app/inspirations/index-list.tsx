"use client"

import { useEffect, useState } from "react"

import { sample } from "../../lib/ramp"

/**
 * The ten headings, as one strip.
 *
 * Read the swatches left to right and you have read the gradient — which is the
 * only place on this site the ramp is used to mean "these are the parts of one
 * thing" rather than "this is how much it costs".
 */
export function Index({ facets }: { facets: { id: string; title: string }[] }) {
  const [here, setHere] = useState(facets[0]?.id)

  useEffect(() => {
    const eye = new IntersectionObserver(
      (entries) => {
        const showing = entries.filter((e) => e.isIntersecting)
        if (showing.length) setHere(showing[0].target.id)
      },
      { rootMargin: "-88px 0px -62% 0px" },
    )
    facets.forEach((f) => {
      const el = document.getElementById(f.id)
      if (el) eye.observe(el)
    })
    return () => eye.disconnect()
  }, [facets])

  return (
    <nav className="ins-index enter" aria-label="The ten headings">
      {facets.map((facet, i) => (
        <a
          key={facet.id}
          href={`#${facet.id}`}
          className={here === facet.id ? "on" : undefined}
          style={{ ["--tone" as string]: sample(i / (facets.length - 1)) }}
        >
          <i />
          <b className="num">{String(i + 1).padStart(2, "0")}</b>
          {facet.title}
        </a>
      ))}
    </nav>
  )
}
