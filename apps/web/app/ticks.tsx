/** Prose with `backticks` in it, rendered as the code it obviously is. */
export function Ticks({ children }: { children: string }) {
  return (
    <>
      {children.split(/(`[^`]+`)/g).map((piece, i) =>
        piece.startsWith("`") && piece.endsWith("`") && piece.length > 2 ? (
          <code key={i}>{piece.slice(1, -1)}</code>
        ) : (
          piece
        ),
      )}
    </>
  )
}
