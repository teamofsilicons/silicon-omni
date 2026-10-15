"use client"

import { useEffect, useRef } from "react"

/**
 * An ordered-dither field, used once per page and never twice.
 *
 * A smooth gradient is quantised to two tones through an 8x8 Bayer matrix, so
 * the falloff is carried by the density of dots rather than by opacity. It is
 * drawn at one device pixel per cell and scaled up with `image-rendering:
 * pixelated`, which is what keeps the dots crisp and the canvas tiny — a
 * 200x120 buffer covers a full-width hero.
 *
 * It sits behind content at low opacity and is skipped entirely on small
 * screens and for anyone who has asked for less motion.
 */

const BAYER = [
  [0, 32, 8, 40, 2, 34, 10, 42],
  [48, 16, 56, 24, 50, 18, 58, 26],
  [12, 44, 4, 36, 14, 46, 6, 38],
  [60, 28, 52, 20, 62, 30, 54, 22],
  [3, 35, 11, 43, 1, 33, 9, 41],
  [51, 19, 59, 27, 49, 17, 57, 25],
  [15, 47, 7, 39, 13, 45, 5, 37],
  [63, 31, 55, 23, 61, 29, 53, 21],
]

type Shape = "radial" | "linear"

export function Dither({
  shape = "radial",
  cells = 190,
  wash = true,
  /** ceiling on the smooth field before it is quantised. Below 1 the pattern
      can never reach solid, which is the difference between a texture and a
      checkerboard. */
  weight = 0.58,
}: {
  shape?: Shape
  cells?: number
  wash?: boolean
  weight?: number
}) {
  const holder = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const box = holder.current
    if (!box) return
    if (window.matchMedia("(max-width: 720px)").matches) return
    // The docstring used to claim this and the code never checked it. It is a
    // still texture, so reduced-motion is not strictly about motion here — but
    // somebody who has asked for less has asked for less.
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return

    const canvas = document.createElement("canvas")
    const brush = canvas.getContext("2d", { willReadFrequently: false })
    if (!brush) return
    box.appendChild(canvas)

    const paint = () => {
      const ratio = box.clientHeight / Math.max(box.clientWidth, 1)
      const w = cells
      const h = Math.max(8, Math.round(cells * ratio))
      canvas.width = w
      canvas.height = h

      const field = brush.createImageData(w, h)
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          // the smooth field this dithers: brightest top-left, falling away
          let value: number
          if (shape === "radial") {
            const dx = x / w - 0.12
            const dy = y / h - 0.05
            value = 1 - Math.min(1, Math.hypot(dx * 1.1, dy * 0.85) * 1.55)
          } else {
            value = 1 - y / h
          }
          value = Math.pow(Math.max(0, value), 1.9) * weight

          // Die out before the canvas does. A CSS mask fades the element, but
          // the element still ends somewhere, and a 1-bit pattern that is still
          // firing when it gets there reads as a cropped rectangle rather than
          // as texture. This makes the last fifth of every edge fall to zero,
          // so the container cannot crop what is no longer being drawn.
          const fade = (a: number, span: number) => Math.min(1, a / Math.max(1, span))
          value *=
            fade(Math.min(x, w - 1 - x), w * 0.2) * fade(Math.min(y, h - 1 - y), h * 0.2)

          // Bayer: compare against a threshold that varies per cell, so a
          // constant value becomes a regular pattern rather than a flat block
          const threshold = (BAYER[y & 7][x & 7] + 0.5) / 64
          const lit = value > threshold
          const at = (y * w + x) * 4
          field.data[at] = 17
          field.data[at + 1] = 17
          field.data[at + 2] = 17
          field.data[at + 3] = lit ? 255 : 0
        }
      }
      brush.putImageData(field, 0, 0)
    }

    paint()
    const watcher = new ResizeObserver(paint)
    watcher.observe(box)
    return () => {
      watcher.disconnect()
      canvas.remove()
    }
  }, [shape, cells, weight])

  return <div className={`dither${wash ? " wash" : ""}`} ref={holder} aria-hidden />
}
