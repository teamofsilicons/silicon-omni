import { ArrowSquareOut, PencilSimple } from "@phosphor-icons/react/dist/ssr"

import { Graph } from "./graph"
import { Chapter } from "../chapter"
import { Dither } from "../dither"
import { Mark } from "../mark"
import { runnable, unusable } from "../../lib/choose"
import { EDIT, graph, VIEW } from "../../lib/graph"
import { Logo, type Vendor } from "../logos"

export const metadata = {
  title: "models — silicon omni",
  description:
    "Omni model configurations, current recommendations, and available benchmark, cost, and timing measurements.",
}

const money = (n: number) => (n >= 1 ? `$${n.toFixed(2)}` : `$${n.toFixed(4)}`)
const clock = (s: number) =>
  s >= 60 ? `${Math.floor(s / 60)}m ${String(Math.round(s % 60)).padStart(2, "0")}s` : `${Math.round(s)}s`
const round = (n: number) => (n >= 100 ? Math.round(n) : Number(n.toFixed(n >= 10 ? 1 : 2)))

const OF = 4

export default async function Models() {
  const { models, benches, keys, source, caveats, fresh } = await graph()
  const names = Object.keys(benches)

  /* Counted, not asserted: coverage is uneven on purpose and the page had
     better say so in numbers rather than in a footnote. */
  const scored = (bench: string) => models.filter((m) => m.scores?.[bench] != null).length

  return (
    <div className="shell rails">
      <section style={{ paddingTop: "4rem", position: "relative" }}>
        <Dither shape="radial" cells={200} />
        <h1 className="rise">The board.</h1>
        <p className="lede column rise" style={{ ["--i" as string]: 1, marginTop: "1.4rem" }}>
          {new Set(models.map(m => m.model)).size} model IDs and {models.length} configurations,
          with available measurements across {names.length} benchmarks. Historical models remain
          visible; only current models enter automatic recommendations. Missing numbers are
          unmeasured, and provider access can vary by account.
        </p>

        <Graph models={models} benches={benches} />

        <div className="column" style={{ marginTop: "2.4rem" }}>
          <p>
            Turn a provider off. The edge moves, and models another vendor was shadowing step
            straight back onto it — which is why the same number means something different to
            somebody signed into fewer things than you.
          </p>
          <p>
            Change the benchmark and the board itself changes shape. A model that owns one is
            mid-table on the next. That is the whole reason the registry asks what you are doing
            before it answers, instead of keeping one league table and calling it intelligence.
          </p>
        </div>
      </section>

      <Chapter
        n={1}
        of={OF}
        name="the shortlists"
        title="Six words, and who answers them."
        argue={
          <p className="set">
            Chosen by hand, best first. The first vendor on a list you are signed into answers, so
            losing one walks you a place down it. We tried working this out arithmetically and it
            produced answers that were defensible and wrong.
          </p>
        }
      >
        <table className="rows wide" style={{ maxWidth: "50rem" }}>
          <thead>
            <tr>
              <th style={{ width: "5.5rem" }}>ask for</th>
              <th style={{ width: "16rem" }}>when</th>
              <th>in order</th>
            </tr>
          </thead>
          <tbody>
            {Object.entries(keys).map(([id, k]) => (
              <tr key={id}>
                <td>
                  <code>{id}</code>
                </td>
                <td className="quiet">{k.says}</td>
                <td>
                  {k.picks.map((m, i) => (
                    <span key={`${m.provider}/${m.model}/${m.effort ?? ""}`}>
                      {i > 0 ? " › " : ""}
                      <span className="step-who" style={{ ["--tone" as string]: `var(--${m.provider}-ink)` }}>
                        <Logo of={m.provider as Vendor} size={11} />
                      </span>
                      {m.model}
                      {m.effort ? <span className="quiet"> {m.effort}</span> : null}
                      {m.fast ? <span className="quiet"> fast</span> : null}
                    </span>
                  ))}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Chapter>

      <Chapter
        n={2}
        of={OF}
        name="the boards"
        title="Five leaderboards, and what each of them has actually measured."
        argue={
          <p className="set">
            A benchmark only earns a place here if somebody published the number. Coverage is
            uneven and stays uneven: a model no board has scored carries no score, and is simply
            not a candidate on that board.
          </p>
        }
      >
        <table className="rows wide" style={{ maxWidth: "50rem" }}>
          <thead>
            <tr>
              <th style={{ width: "6rem" }}>ask for</th>
              <th>leaderboard</th>
              <th style={{ width: "7rem" }}>scored</th>
            </tr>
          </thead>
          <tbody>
            {names.map((id) => (
              <tr key={id}>
                <td>
                  <code>{id}</code>
                </td>
                <td>
                  <a className="link" href={benches[id].url}>
                    {benches[id].label}
                  </a>
                </td>
                <td className="n">
                  {scored(id)} of {models.length}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Chapter>

      <Chapter
        n={3}
        of={OF}
        name="every row"
        title="The database, as it stands this second."
        argue={
          <p className="set">
            One row per model <em>and</em> effort, because effort changes all three numbers. A dash
            is a board that has not scored that model, and it is left as a dash rather than
            estimated onto the chart.
          </p>
        }
      >
        <div style={{ overflowX: "auto" }}>
          <table className="rows wide" style={{ minWidth: "44rem" }}>
            <thead>
              <tr>
                <th>model</th>
                {names.map((id) => (
                  <th key={id} style={{ width: "4.6rem" }}>
                    {id}
                  </th>
                ))}
                <th style={{ width: "5.4rem" }}>$/task</th>
                <th style={{ width: "5rem" }}>time</th>
              </tr>
            </thead>
            <tbody>
              {models.map((m) => (
                <tr key={`${m.provider}/${m.model}/${m.effort}`}>
                  <td>
                    <span className="step-who" style={{ ["--tone" as string]: `var(--${m.provider}-ink)` }}>
                      <Logo of={m.provider as Vendor} size={11} />
                    </span>
                    {m.model} <span className="quiet">{m.effort}</span>
                  </td>
                  {names.map((id) => (
                    <td key={id} className="n">
                      {m.scores?.[id] != null ? round(m.scores[id] as number) : "—"}
                    </td>
                  ))}
                  <td className="n">{m.price == null ? "—" : money(m.price)}</td>
                  <td className="n">{m.secs == null ? "—" : clock(m.secs)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Chapter>

      <Chapter
        n={4}
        of={OF}
        name="the source"
        title="The whole database is one file."
        argue={
          <p className="set">
            Administrators sign in with Silicon Accounts, edit model configurations and category
            recommendations, and publish. Earlier catalog versions are retained, and clients
            receive updates without a library release or website deployment.
          </p>
        }
      >
        <div className="column">
          <p>
            Read the public{" "}
            <a className="link" href={VIEW}>
              models.json
            </a>{" "}
            catalog, or use the admin editor to publish a change. This page reads live data;
            clients refresh their cached recommendations within the hour.{" "}
            {fresh ? null : (
              <strong>
                This local preview uses the bundled seed catalog.
              </strong>
            )}
          </p>
          <ul className="caveats">
            {caveats.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
          <div className="row" style={{ marginTop: "1.6rem" }}>
            <a className="btn solid" href={EDIT}>
              <PencilSimple size={14} weight="bold" />
              add a model
            </a>
            <a className="btn ghost" href="/choose.json?bench=business&intelligence=5&cost=2&speed=10">
              the answer, as json
              <ArrowSquareOut size={14} weight="bold" />
            </a>
            <a className="btn ghost" href="/registry.json">
              what you may ask for
              <ArrowSquareOut size={14} weight="bold" />
            </a>
          </div>
        </div>
      </Chapter>

      <div className="breath">
        <Mark h={16} />
      </div>
    </div>
  )
}
