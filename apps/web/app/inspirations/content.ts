/**
 * Where this came from.
 *
 * Design is not only how a thing looks. It is how it feels, how it is
 * understood, how it travels, how it is used over time, what idea it carries,
 * how it reads, how it is laid out, how it moves and how it answers to a hand.
 * Ten headings, then, and six or seven debts under each. The page counts them
 * itself rather than printing a number that can rot.
 *
 * Every entry was checked. `mechanic` is what the thing actually does, stated
 * concretely enough to argue with. `taken` is what we did about it — and that
 * half is ours, so if a connection seems far-fetched, it is, and it was still
 * the reason.
 */

export interface Note {
  name: string
  maker: string
  year: string
  url: string
  mechanic: string
  taken: string
}

export interface Facet {
  id: string
  title: string
  /** the running head, which is not always the title with a word knocked off */
  short: string
  blurb: string
  notes: Note[]
}

export const FACETS: Facet[] = [
  {
    id: "looks",
    short: "looks",
    title: "How it looks",
    blurb:
      "The surface. Warm paper, one ink, and colour held back until it has something to count.",
    notes: [
      {
        name: "Isotype, the Vienna Method of Pictorial Statistics",
        maker: "Otto Neurath, Gerd Arntz and Marie Reidemeister",
        year: "1925–34",
        url: "http://gerdarntz.org/content/gerd-arntz.html",
        mechanic:
          "More of a thing is never a bigger symbol. It is more symbols of one fixed value, so a magnitude is counted rather than guessed — and Arntz cut some four thousand of them to keep the vocabulary fixed instead of invented per chart.",
        taken:
          "A conversation is measured in identical countable units — one turn, one increment — rather than in tokens or bytes, so three vendors land on one axis.",
      },
      {
        name: "World Geo-Graphic Atlas",
        maker: "Herbert Bayer for Container Corporation of America",
        year: "1953",
        url: "https://archive.org/details/dr_title-page-world-geo-graphic-atlas-a-composite-of-mans-environment-geo-5842004",
        mechanic:
          "One man edited, wrote and drew the whole book, forcing six unrelated disciplines through a single grid, symbol set and diagram vocabulary.",
        taken:
          "One dataclass carries every event from all three CLIs, so the file on disk and the objects your handler receives are the same schema and never two.",
      },
      {
        name: "Tonhalle Zürich concert posters",
        maker: "Josef Müller-Brockmann",
        year: "from 1950",
        url: "https://en.wikipedia.org/wiki/Josef_M%C3%BCller-Brockmann",
        mechanic:
          "No photograph of the orchestra and no illustration — the concert's facts, plus geometric primitives sized and placed by a modular grid rather than by eye.",
        taken:
          "The eleven rungs are placed by the frontier. That is why levels are allowed to share a rung when the edge is short, instead of being spaced out to look full.",
      },
      {
        name: "Topographic Map Symbols",
        maker: "United States Geological Survey",
        year: "2005 edition",
        url: "https://pubs.usgs.gov/gip/TopographicMapSymbols/topomapsymbols.pdf",
        mechanic:
          "Colour denotes class, not appearance: contours brown, water blue, land grids red, cultural features black, vegetation green — the same ink means the same thing on every quadrangle in the country.",
        taken:
          "One stop of the ramp per vendor and the whole ramp for the dial. A colour on this site is a claim about what kind of thing you are looking at.",
      },
      {
        name: "Kentō registration marks in woodblock printing",
        maker: "Edo-period block cutters",
        year: "in use by the mid-18th century",
        url: "https://pulverer.si.edu/node/1217",
        mechanic:
          "Two notches cut into every block of a set — a right-angled key at the bottom right, a straight drop-stop at the left — so one sheet can be laid onto block after block in register. Full-colour printing is downstream of that notch.",
        taken:
          "Each vendor's turn is a separate pass over one sheet, and the record of how far up the log each has read is the notch that makes the passes land in register.",
      },
      {
        name: "Nuova pianta di Roma",
        maker: "Giambattista Nolli",
        year: "1748",
        url: "https://www.clevelandart.org/art/2020.276.6",
        mechanic:
          "The city cut as one slice at ground level across twelve copperplates, with the interiors of churches and colonnades drawn out in full plan while private fabric stays a filled black block.",
        taken:
          "State another library would keep as an opaque vendor session is a readable file in your home directory. Anything shared has its interior drawn out.",
      },
    ],
  },
  {
    id: "feels",
    short: "feels",
    title: "How it feels",
    blurb: "Temperature. What a surface is like to be near when you are tired.",
    notes: [
      {
        name: "The Doves Press",
        maker: "T. J. Cobden-Sanderson with Emery Walker",
        year: "from 1900",
        url: "https://www.emerywalker.org.uk/doves-press",
        mechanic:
          "One typeface, one size, no illustrations — leaving paper, spacing and impression as the only variables. When the partnership failed, Cobden-Sanderson made around 170 night trips to Hammersmith Bridge and dropped over a ton of type into the Thames.",
        taken:
          "Options are removed on purpose: one session, one file, one dial, zero Python dependencies. What is left of the craft is spacing — what gets written down, and when.",
      },
      {
        name: "Shaker furniture and the Millennial Laws",
        maker: "The United Society of Believers",
        year: "19th century",
        url: "https://en.wikipedia.org/wiki/Shaker_furniture",
        mechanic:
          "A written code reached down to the colours furniture could be painted, and barred inlay, carving, veneer and metal pulls as prideful — which pushed all variety into asymmetric drawer arrangements and multipurpose forms.",
        taken:
          "The refusals are published as part of the surface, so the argument about ornament is settled once in the rules rather than at every call site.",
      },
      {
        name: "Apollo 11 Flight Plan, final version",
        maker: "Flight Planning Branch, NASA Manned Spacecraft Center",
        year: "1 July 1969",
        url: "https://archive.org/details/apollo-11-flight-plan",
        mechanic:
          "Three hundred and fifty-three pages schedule an entire mission with no adjectives in it anywhere, every crew activity written as a line against its Ground Elapsed Time.",
        taken:
          "Whatever the three CLIs actually said, a failure is reported as one of four plain outcomes — log in, wait, retry, or read a stack trace. Somebody reading under load needs a verb, not a vendor's phrasing.",
      },
      {
        name: "The Edmondson railway ticket",
        maker: "Thomas Edmondson, Manchester & Leeds Railway",
        year: "1840s",
        url: "https://en.wikipedia.org/wiki/Edmondson_railway_ticket",
        mechanic:
          "Every ticket the same card, individually numbered within its series and date-stamped at the moment of sale, with the lowest unsold number of each type left visible in a lockable rack.",
        taken:
          "A conversation audits like a ticket rack. Because every turn is stamped with its own number, you count what is there and the gaps are the finding.",
      },
      {
        name: "Shikinen Sengū at Ise Jingū",
        maker: "Ise Jingū",
        year: "sixty-second rebuilding, 2013",
        url: "https://www.isejingu.or.jp/en/ritual/index.html",
        mechanic:
          "Every twenty years a new sanctuary of identical dimensions is built on the adjacent site, the treasures are remade, the deity is moved, and the old building comes down.",
        taken:
          "Continuity lives in the file and not the process, so omni can close stdin on one CLI, terminate it, and raise another on the adjacent plot with nothing carried across but the log.",
      },
      {
        name: "The SBB station clock",
        maker: "Hans Hilfiker with Moser-Baer",
        year: "1944; red hand added 1953",
        url: "https://en.wikipedia.org/wiki/Swiss_railway_clock",
        mechanic:
          "The second hand sweeps its circle in about 58.5 seconds, then waits at twelve for the minute impulse from a central master clock, so every clock on the network turns the minute at the same instant.",
        taken:
          "One authority defines the tick — omni's own counter, never a vendor's — so three CLIs answering at three latencies still agree on which turn it is.",
      },
    ],
  },
  {
    id: "understood",
    short: "understood",
    title: "How it is understood",
    blurb: "The moment a thing clicks, and what it costs to build that moment.",
    notes: [
      {
        name: "Mechanical Watch",
        maker: "Bartosz Ciechanowski",
        year: "2022",
        url: "https://ciechanow.ski/mechanical-watch/",
        mechanic:
          "Every figure is a live model you drag rather than a picture, and each part is introduced in the order that creates the next problem — mainspring, gear train, then the escapement the balance wheel unlocks one beat at a time.",
        taken:
          "The log on this page plays itself and can also be stepped one line at a time, because a mechanism you cannot stop mid-stroke is a mechanism you have to take on trust.",
      },
      {
        name: "Parable of the Polygons",
        maker: "Vi Hart and Nicky Case",
        year: "2014",
        url: "https://ncase.me/polygons/",
        mechanic:
          "You are handed the actual rule — a shape is unhappy when fewer than a third of its neighbours match — and must drag unhappy shapes by hand until the board settles, so you produce the result before the text names it.",
        taken:
          "The graph page lets you switch a provider off and watch the rungs move. It is the only honest way to explain that the same number means different things to different people.",
      },
      {
        name: "Up and Down the Ladder of Abstraction",
        maker: "Bret Victor",
        year: "2011",
        url: "https://worrydream.com/LadderOfAbstraction/",
        mechanic:
          "One toy system is climbed rung by rung, each step trading a concrete detail for an added dimension, with an explicit move back down to a single case at every level.",
        taken:
          "The dial is one rung, not the only rung. The model name and its measured price stay addressable underneath, because a library that removes the lower rung has stranded you at the altitude it chose.",
      },
      {
        name: "The Elements of Euclid, in colour",
        maker: "Oliver Byrne",
        year: "1847",
        url: "https://publicdomainreview.org/collection/the-first-six-books-of-the-elements-of-euclid-1847/",
        mechanic:
          "Byrne deleted Euclid's alphabetic labels and set the figures themselves into the sentences, printing every line and angle in red, yellow, blue or black — so a proof is read off shapes with no lookup from “AB” back to the diagram.",
        taken:
          "Three CLIs are addressed through one session API — as a Python object, a Rust client or a terminal command — rather than through each vendor's flags and vocabulary. The indirection is the thing deleted, not the thing added.",
      },
      {
        name: "Powers of Ten",
        maker: "Charles and Ray Eames",
        year: "1977",
        url: "https://www.eamesfoundation.org/work/powers-of-ten-and-the-relative-size-of-things-in-the-universe/",
        mechanic:
          "Nine minutes governed by one fixed exchange rate: the field of view multiplies by ten every ten seconds outward, then contracts by a power of ten every two seconds inward. Scale is carried by the rate, not by captions.",
        taken:
          "The dial is one monotonic axis with one rule for every step — down and to the left, never sideways — so what you learn going from 3 to 5 transfers to going from 7 to 9.",
      },
      {
        name: "Feynman diagrams",
        maker: "Richard Feynman",
        year: "1948",
        url: "https://en.wikipedia.org/wiki/Feynman_diagram",
        mechanic:
          "A picture with a strict grammar stands in for an integral: straight lines fermions, wavy lines bosons, vertices interactions, the past at the bottom — and every diagram is exactly one term in a series.",
        taken:
          "A tool the destination cannot run is rewritten as a line that reads as what happened, while the structured original stays in the log. The picture is for the reader; the term underneath is still exact.",
      },
    ],
  },
  {
    id: "shared",
    short: "shared",
    title: "How it is shared",
    blurb: "The mechanics of a thing travelling further than the person who made it.",
    notes: [
      {
        name: "The Penny Black and the Uniform Penny Post",
        maker: "Rowland Hill and the General Post Office",
        year: "issued 1 May 1840",
        url: "https://en.wikipedia.org/wiki/Penny_Black",
        mechanic:
          "One penny, prepaid by the sender, carried a letter anywhere in the United Kingdom regardless of distance — replacing a tariff computed by sheet and mileage and collected on delivery.",
        taken:
          "The level is committed to before the turn runs, not reconciled after it, which is why the frontier is computed into eleven fixed stops rather than priced live per request.",
      },
      {
        name: "The Treaty of Bern, founding the General Postal Union",
        maker: "22 nations at Bern",
        year: "9 October 1874",
        url: "https://www.upu.int/en/universal-postal-union/about-upu/history",
        mechanic:
          "A lattice of bilateral agreements collapsed into a single postal territory in which each administration carries another's mail onward as though it were its own.",
        taken:
          "The transcript format is the treaty and the vendors are only administrations. A turn written while one CLI was driving has to be readable by the other two with nothing negotiated between them.",
      },
      {
        name: "Whole Earth Catalog",
        maker: "Stewart Brand, Portola Institute",
        year: "Fall 1968",
        url: "https://en.wikipedia.org/wiki/Whole_Earth_Catalog",
        mechanic:
          "It sold nothing. It stated its own function — “an evaluation and access device” — and admitted an item only if it was useful as a tool, relevant to independent education, high quality or low cost, not already common knowledge, and available by mail.",
        taken:
          "The registry is an access device too. It lists models and ships none of them, and a model earns a rung only by sitting on the frontier — which is those five tests restated for tokens.",
      },
      {
        name: "Species Plantarum and binomial nomenclature",
        maker: "Carl Linnaeus",
        year: "1 May 1753",
        url: "https://en.wikipedia.org/wiki/Species_Plantarum",
        mechanic:
          "Plants lost their descriptive polynomials for a two-word handle, and one book was later fixed as the starting point for priority, so no earlier name can be produced to unseat a later one.",
        taken:
          "A turn's position is settled once, at write time, so a vendor that re-emits or renumbers its own messages cannot reorder the conversation retroactively.",
      },
      {
        name: "The ISBN",
        maker: "Gordon Foster for W H Smith",
        year: "1965; ISO 2108 in 1970",
        url: "https://en.wikipedia.org/wiki/ISBN",
        mechanic:
          "Issued by a registry rather than by the publisher, with a check digit so a mistyped number fails outright instead of quietly resolving to a different book — and a separate number for every edition and format.",
        taken:
          "The database holds one row per model and effort, because effort changes both what a model scores and what it costs. Collapsing them would be an ebook and a hardback sharing an ISBN.",
      },
      {
        name: "WikiWikiWeb",
        maker: "Ward Cunningham",
        year: "25 March 1995",
        url: "http://wiki.c2.com/?WikiWikiWeb",
        mechanic:
          "Any reader could edit any page from the browser, and a link pointing at a page that did not yet exist became the invitation to write it.",
        taken:
          "The page and not the author is the unit of identity. The file is the conversation, and whichever of the three tools spoke last is only the most recent hand to have edited it.",
      },
    ],
  },
  {
    id: "experienced",
    short: "experienced",
    title: "How it is experienced",
    blurb: "Over time, through use, at three in the morning.",
    notes: [
      {
        name: "STCW Code, Section A-VIII/2, Part 4-1, paragraph 23",
        maker: "International Maritime Organization, the Manila amendments",
        year: "2010, in force 2012",
        url: "https://wwwcdn.imo.org/localresources/en/OurWork/HumanElement/Documents/34.pdf",
        mechanic:
          "A watch handover is not merely scheduled — it is forbidden mid-manoeuvre. Paragraph 23 defers the relief of the officer of the watch until the action being taken to avoid a hazard is complete. It is quoted in full in chapter six, which is named after the distinction underneath it: under pilotage the pilot has conduct of the vessel and the master never stops being in command.",
        taken:
          "The turn boundary, written down by an industry that learned it by drowning people. Everything omni holds until the seam is held for this reason, and everything it injects immediately is injected because a message is not a manoeuvre.",
      },
      {
        name: "The Beaufort wind scale",
        maker: "Francis Beaufort",
        year: "1805",
        url: "https://en.wikipedia.org/wiki/Beaufort_scale",
        mechanic:
          "Thirteen steps defined not by a measured speed but by what a fully rigged man-of-war could carry at that force. No anemometer readings were attached to the numbers until 1923.",
        taken:
          "The dial names a working condition rather than a measurement, which is the only reason the same number stays meaningful after the model underneath it has changed.",
      },
      {
        name: "The Caen Hill lock flight, Kennet & Avon Canal",
        maker: "John Rennie the Elder",
        year: "opened 1810",
        url: "https://en.wikipedia.org/wiki/Caen_Hill_Locks",
        mechanic:
          "Twenty-nine locks lift a boat 237 feet in two miles. The pounds between the middle sixteen are too short to hold the water a chamber needs, so fifteen carry oversized side pounds that bank it in advance. A boat takes five or six hours.",
        taken:
          "A switch is not instant and does not pretend to be. It is recorded the moment you ask and applied at the next turn boundary, once the running tool has finished.",
      },
      {
        name: "The London Underground diagram",
        maker: "Harry Beck",
        year: "published 1933",
        url: "https://www.londonmuseum.org.uk/collections/london-stories/harry-beck-revolutionised-tube-map/",
        mechanic:
          "A junior draughtsman from the signal engineers' office discarded geographic scale and drew the network with horizontals, verticals and 45-degree diagonals, enlarging the centre so the names would fit.",
        taken:
          "The transcript makes the same deliberate distortion: it keeps the sequence and throws away the topology, because a reader needs to know where the thread went next, not which process was on the wire.",
      },
      {
        name: "Sallie Gardner at a Gallop",
        maker: "Eadweard Muybridge",
        year: "19 June 1878",
        url: "https://spectrum.ieee.org/june-1878-muybridge-photographs-a-galloping-horse",
        mechanic:
          "A line of cameras beside the Palo Alto track, each shutter tripped by a thread the passing horse broke — so the sequence is indexed by position along the track rather than by any clock.",
        taken:
          "Turns are indexed by an integer and not a timestamp, because the wall-clock timings three different vendor CLIs report are not comparable to each other.",
      },
      {
        name: "The organoleptic dilution test for capsicum",
        maker: "Wilbur L. Scoville",
        year: "1912",
        url: "https://en.wikipedia.org/wiki/Scoville_scale",
        mechanic:
          "Ground chilli is diluted in sugar water until a panel can no longer detect heat, and the reported figure is that dilution ratio — a threshold of human perception, not a chemical assay.",
        taken:
          "Eleven integer stops rather than a continuous score. Resolution finer than an operator can feel is reported precision that buys nothing.",
      },
      {
        name: "The rock garden at Ryōan-ji",
        maker: "attribution disputed",
        year: "probably late 15th century",
        url: "https://en.wikipedia.org/wiki/Ry%C5%8Dan-ji",
        mechanic:
          "Fifteen stones in five groups, arranged so that from the viewing veranda at least one is always hidden behind another. There is no position from which you see all fifteen.",
        taken:
          "There is no single vantage point, so the registry answers with every combination of providers at once — one ladder per possible install.",
      },
    ],
  },
  {
    id: "conveyed",
    short: "the idea",
    title: "Ideas conveyed",
    blurb: "The argument under the surface, and who made it first.",
    notes: [
      {
        name: "Lloyd's Register, and the Red Book against the Green Book",
        maker: "The Register Society, London",
        year: "register from 1764; the schism 1800–33; merged 1834",
        url: "https://www.lr.org/en/who-we-are/brief-history/",
        mechanic:
          "Hulls graded by letter and fittings by number — hence A1 — in a book published for the underwriters and merchants deciding whether to insure a ship they had never seen. The schism and the merger are in chapter eight; what belongs here is the grading scheme itself, which puts two independent measures on one object and refuses to average them into a score.",
        taken:
          "This is why the dial lives on a third-party site reading a third-party benchmark, and not inside any vendor's SDK. Neutrality here is not a principle anybody was feeling noble about; it is the only configuration that has ever survived being useful to two parties at once.",
      },
      {
        name: "Double-entry bookkeeping",
        maker: "Luca Pacioli, Summa de arithmetica, Venice",
        year: "1494",
        url: "https://www.icaew.com/library/library-collection/historical-accounting-literature/highlights-of-the-collection/summa-di-arithmetica",
        mechanic:
          "Every transaction entered twice, once as a debit and once as a matching credit, so the books can be proved arithmetically rather than vouched for by the bookkeeper.",
        taken:
          "omni owns the transcript rather than asking each CLI for its own history, because a record is only checkable if the party being checked did not write it.",
      },
      {
        name: "Pareto optimality",
        maker: "Vilfredo Pareto, Manuale di economia politica",
        year: "1906",
        url: "https://en.wikipedia.org/wiki/Pareto_efficiency",
        mechanic:
          "An arrangement is optimal when no change can improve one measure without worsening another. The optimal arrangements form a boundary, and everything behind it is beaten on both measures at once.",
        taken:
          "The eleven stops are that boundary in the plane of score against cost. Everything behind the edge is excluded by construction, and an unmeasured model is left off rather than estimated on.",
      },
      {
        name: "The Plimsoll load line",
        maker: "Samuel Plimsoll, Merchant Shipping Act",
        year: "1876; position fixed in law 1894",
        url: "https://www.rmg.co.uk/stories/maritime-history/samuel-plimsoll-ship-safety",
        mechanic:
          "A twelve-inch circle with a line through it, painted on the hull and made compulsory by statute, turned overloading from a dispute about tonnage into a fact visible from the quayside.",
        taken:
          "One visible mark beats a calculation every operator performs differently. That is the whole case for one dial instead of a bag of per-vendor knobs.",
      },
      {
        name: "The standardised shipping container",
        maker: "Malcom McLean, SS Ideal X",
        year: "26 April 1956",
        url: "https://porthouston.com/ideal-x/",
        mechanic:
          "Fifty-eight identical boxes craned onto a converted tanker. Because the fitting between box and crane was fixed in advance, loading fell from $5.86 a ton to about sixteen cents — and what was inside stopped mattering to the ship.",
        taken:
          "Three rival CLIs can be swapped mid-conversation precisely because none of them is allowed to define the container the turns travel in.",
      },
      {
        name: "The Unix pipe",
        maker: "M. D. McIlroy's idea, implemented by Dennis Ritchie",
        year: "1972",
        url: "https://www.nokia.com/bell-labs/about/dennis-m-ritchie/hist.html",
        mechanic:
          "One program's standard output feeds straight into the next program's standard input, and neither contains any knowledge that the other exists.",
        taken:
          "The backends are addressed only through the transcript, so adding a fourth vendor is a matter of writing into the same stream rather than teaching the existing three about the newcomer.",
      },
      {
        name: "Time, Clocks, and the Ordering of Events in a Distributed System",
        maker: "Leslie Lamport, Communications of the ACM 21(7)",
        year: "July 1978",
        url: "https://lamport.azurewebsites.net/pubs/time-clocks.pdf",
        mechanic:
          "Each process holds a counter that only increases, stamps every event with it, and raises it on receipt of a message — yielding one total order over events without any process trusting a physical clock.",
        taken:
          "The counter is monotone and owned by the library, and the meta file records how far up it each vendor has read. Three CLIs with three notions of “when” still interleave into one thread.",
      },
    ],
  },
  {
    id: "reads",
    short: "reads",
    title: "How it reads",
    blurb:
      "The sentence. Every one of these is here because somebody refused an adjective and used a fact instead.",
    notes: [
      {
        name: "“This Volkswagen missed the boat.”",
        maker: "Volkswagen “Lemon”, Doyle Dane Bernbach",
        year: "1960",
        url: "https://www.manifestowriting.com/database/volkswagen-lemon/",
        mechanic:
          "The copy convicts its own product in five plain words, then spends the remaining ninety turning that admission into the evidence for the claim, closing on “We pluck the lemons; you get the plums.”",
        taken:
          "The refusals are a chapter, not a footnote. Reasoning cannot cross a switch, Antigravity cannot be muzzled, an unmeasured model stays off the dial.",
      },
      {
        name: "“At 60 miles an hour the loudest noise in this new Rolls-Royce comes from the electric clock”",
        maker: "David Ogilvy",
        year: "1958",
        url: "https://en.wikipedia.org/wiki/David_Ogilvy_(businessman)",
        mechanic:
          "Not one adjective of praise. The sentence supplies a measured condition and one observed detail, and leaves the reader to compute quietness for themselves.",
        taken:
          "Each rung is documented by its two numbers and never by a word like smart, fast or best.",
      },
      {
        name: "“Houston, Tranquility Base here. The Eagle has landed.”",
        maker: "Neil Armstrong, Apollo 11 air-to-ground, GET 102:45:58",
        year: "20 July 1969",
        url: "https://history.nasa.gov/wp-content/uploads/static/history/alsj/a11/a11.landing.html",
        mechanic:
          "Every line stamped with Ground Elapsed Time — one clock counting up from launch — so a lunar module, a command module and a room in Houston write into one strictly ordered record.",
        taken:
          "The sequence number is omni's Ground Elapsed Time. It counts from the start of the conversation, not from the start of any vendor's session.",
      },
      {
        name: "“And God said, Let there be light: and there was light.”",
        maker: "The King James translators, Genesis 1:3",
        year: "1611",
        url: "https://en.wikisource.org/wiki/Bible_(King_James)/Genesis",
        mechanic:
          "Eleven words, every one a monosyllable, hinged on a colon that puts command and outcome in one sentence with no clause to explain or connect them.",
        taken:
          "The public call has the same two halves and nothing in between: you send the turn and the result arrives. No client to construct, no key to hold.",
      },
      {
        name: "“Break any of these rules sooner than say anything outright barbarous.”",
        maker: "George Orwell, “Politics and the English Language”, rule vi",
        year: "1946",
        url: "https://www.orwellfoundation.com/the-orwell-foundation/orwell/essays-and-other-works/politics-and-the-english-language/",
        mechanic:
          "Five flat prohibitions, then a sixth that outranks all five — so the list admits its own incompleteness in the same breath it is issued.",
        taken:
          "The dial is rules one to five, and naming a model outright is rule six. A frontier computed last month is exactly the case the rules were not written for.",
      },
      {
        name: "The Preamble to the Constitution of the United States",
        maker: "The Federal Convention",
        year: "17 September 1787",
        url: "https://www.archives.gov/founding-docs/constitution-transcript",
        mechanic:
          "One sentence carries a subject, six parallel purpose clauses, and then a single operative verb — so everything the document intends is declared before anything it does.",
        taken:
          "The package exports one name, and three vendors, one file, one sequence and one dial all hang off it the way six purposes hang off “do ordain and establish”.",
      },
    ],
  },
  {
    id: "laid-out",
    short: "laid out",
    title: "How it is laid out",
    blurb: "Grid, column, margin. The geography of a page and who is allowed to set it.",
    notes: [
      {
        name: "Penguin Composition Rules",
        maker: "Jan Tschichold, Penguin Books",
        year: "1947",
        url: "https://en.wikipedia.org/wiki/Penguin_Composition_Rules",
        mechanic:
          "A four-page booklet pushed the standard down to letterfit — Tschichold had a rubber stamp made reading “Equalize letter-spaces according to their visual value” — so printers he never met set a Penguin page identically.",
        taken:
          "The three CLIs are the outside compositors. omni hands them a fixed transcript format instead of trying to govern what happens inside a vendor's process.",
      },
      {
        name: "Designing Programmes",
        maker: "Karl Gerstner, Arthur Niggli, Teufen",
        year: "1964",
        url: "https://www.lars-mueller-publishers.com/designing-programmes",
        mechanic:
          "Four essays replace the finished layout with a programme — a declared set of parameters whose permutations are all valid outcomes — so the real work is choosing the axes, not drawing the page.",
        taken:
          "The dial is a programme in exactly this sense. The two axes are published and the eleven stops are whatever the frontier returns, which is why the ladder can change without a release.",
      },
      {
        name: "NYCTA Graphics Standards Manual",
        maker: "Massimo Vignelli and Bob Noorda, Unimark International",
        year: "1970",
        url: "https://standardsmanual.com/products/nyctacompactedition",
        mechanic:
          "Issued on square 13-inch sheets, it specifies each sign as a dimensioned diagram, so a fabricator could build a correct sign without ever seeing the station it was for.",
        taken:
          "What is standardised is the sign, not the sign-maker. The model string goes to the CLI verbatim: whatever a future model is called, that is how it gets run.",
      },
      {
        name: "The data portraits of “The Exhibit of American Negroes”",
        maker: "W. E. B. Du Bois and his Atlanta University students, Exposition Universelle, Paris",
        year: "1900",
        url: "https://en.wikipedia.org/wiki/The_Exhibit_of_American_Negroes",
        mechanic:
          "Two sets of charts — thirty-two on “The Georgia Negro” plus around thirty more — put spirals, folded bars and stacked blocks inside one hand-lettered visual system, so the eye compares data rather than re-learning the frame.",
        taken:
          "Every vendor's turn lands in the same plate: one record shape with a field naming which CLI produced it, so reading the transcript is comparing turns rather than comparing three log formats.",
      },
      {
        name: "The Bomberg page of the Babylonian Talmud",
        maker: "Daniel Bomberg, Venice, adopting Soncino's layout",
        year: "1519–23",
        url: "https://en.wikipedia.org/wiki/Daniel_Bomberg",
        mechanic:
          "Text in the middle, Rashi on the inner margin, Tosafot on the outer — and Bomberg's foliation became the pagination every later edition from any printer has had to reproduce.",
        taken:
          "Because omni owns the pagination and not the vendor, a reference to turn 41 resolves to the same turn whichever of the three tools happened to set it.",
      },
      {
        name: "DIN 476, the A-series paper format",
        maker: "Walter Porstmann, from Lichtenberg's observation of 1786",
        year: "1922",
        url: "https://www.cl.cam.ac.uk/~mgk25/iso-paper.html",
        mechanic:
          "Two decisions — height over width is the square root of two, and A0 is one square metre — make every size a halving of the one before, so the proportion survives every cut.",
        taken:
          "The dial preserves proportion the same way: moving from 3 to 7 changes the model and the bill but not the call, the file format or the sequence.",
      },
    ],
  },
  {
    id: "moves",
    short: "moves",
    title: "How it moves",
    blurb: "Timing. Who decides the interval, and what it costs to be wrong about it.",
    notes: [
      {
        name: "The fusil photographique",
        maker: "Étienne-Jules Marey",
        year: "1882",
        url: "https://en.wikipedia.org/wiki/%C3%89tienne-Jules_Marey",
        mechanic:
          "A clockwork disc steps twelve glass frames past one rotating slit shutter every second, so each phase of a wingbeat lands at an interval the machine chose rather than one the operator did.",
        taken:
          "omni owns the plate and not the camera. Changing vendor changes the shutter and leaves the exposure intact.",
      },
      {
        name: "Pas de deux",
        maker: "Norman McLaren, National Film Board of Canada",
        year: "1968",
        url: "https://www.nfb.ca/film/pas_de_deux/",
        mechanic:
          "An optical printer re-shoots the same high-contrast footage in staggered copies, up to ten deep, so one frame carries the dancer's current position and nine earlier ones at once.",
        taken:
          "This is the argument for replaying the accumulated log into a newly arrived CLI rather than a summary of it. The incoming vendor is handed the earlier positions, not a description of them.",
      },
      {
        name: "The Cifra 5 split-flap clock",
        maker: "Gino Valle for Solari di Udine",
        year: "1954; Compasso d'Oro 1956",
        url: "https://en.wikipedia.org/wiki/Solari_di_Udine",
        mechanic:
          "Each digit is a set of half-cards on a drum that turns one way only, so reaching a new number means dropping every card between here and there. There is no path backwards.",
        taken:
          "A correction is appended as a later numbered turn instead of an edit to an earlier one, which is what lets the log be replayed up to any point without reconstructing history.",
      },
      {
        name: "The metronome",
        maker: "Johann Nepomuk Maelzel, English patent",
        year: "1815",
        url: "https://en.wikipedia.org/wiki/Metronome",
        mechanic:
          "A weight slid along an inverted pendulum sets the beat on a graduated scale running 40 to 208, whose steps widen from 2 to 8 as the tempo rises.",
        taken:
          "The dial is deliberately non-linear in money — the low stops crowd together and the top ones jump — so the cost bars are drawn on a log scale and one click is never one unit of spend.",
      },
      {
        name: "Accumulation",
        maker: "Trisha Brown",
        year: "1971",
        url: "https://en.wikipedia.org/wiki/Trisha_Brown",
        mechanic:
          "The dancer performs gesture one, then one and two, then one, two and three — so the work can never be extended without being restarted from its first movement.",
        taken:
          "Arriving at a new provider means replaying from the top of what it missed, and that cost is exactly what buys a change of dancer mid-phrase. Staying put uses the vendor's own resume and never reads the log at all.",
      },
      {
        name: "Rhythmic Characters of Lights, IALA Recommendation E-110",
        maker: "International Association of Marine Aids to Navigation",
        year: "edition 4, 2016",
        url: "https://www.iala.int/content/uploads/2017/03/E-110-Ed.4-Rhythmic-Characters-of-Lights-on-Aids-to-Navigation_16Dec2016.pdf",
        mechanic:
          "Every navigational light gets a fixed number of flashes inside a fixed period, written on the chart as something like Fl(3) 10s — so a mariner identifies a light by timing it with a watch, not by looking harder.",
        taken:
          "The producing vendor is stamped onto each turn as it is written, so provenance in a mixed conversation is read off the file rather than guessed from the prose.",
      },
    ],
  },
  {
    id: "interacts",
    short: "interacts",
    title: "How it interacts",
    blurb: "What a control is like under a hand, and what it refuses to let you do.",
    notes: [
      {
        name: "Tyer's Electric Train Tablet",
        maker: "Edward Tyer",
        year: "patented 1878",
        url: "https://www.gracesguide.co.uk/Edward_Tyer",
        mechanic:
          "A pair of instruments, one at each end of a single-line section, holds identical tablets. A tablet comes out only if both signalmen agree to release it, and once one is out no other can be withdrawn from either instrument until that one is put back. Tyer patented it four years after the Thorpe collision of 10 September 1874, in which twenty-five people died on a single line because a clerk sent a crossing order before it had been signed.",
        taken:
          "The daemon registry is the pair of instruments. One live engine owns a session id while any number of clients may attach to it, and a per-id lifecycle gate serializes open, send, setting, stop and reap decisions so two engines can never be released onto the same line.",
      },
      {
        name: "Beomaster 1900",
        maker: "Jacob Jensen for Bang & Olufsen",
        year: "1976",
        url: "https://beoworld.org/beomaster-1900/",
        mechanic:
          "Volume is set by resting a finger on a printed dimple until the level arrives, while presets, treble, bass and balance all sit on sliders under an aluminium lid that has to be lifted first.",
        taken:
          "The front face is one dial from 0 to 10. Vendor choice, model names and per-CLI flags stay reachable on the object but off the surface: the common gesture needs no lid, the rare one needs a deliberate lift.",
      },
      {
        name: "The double escapement grand piano action",
        maker: "Sébastien Érard, English patent no. 4,631",
        year: "1821",
        url: "https://en.wikipedia.org/wiki/S%C3%A9bastien_%C3%89rard",
        mechanic:
          "A repetition lever catches the falling hammer and holds it waiting under the string, so the same note can be struck again from a key that has risen only part of the way.",
        taken:
          "A retry re-sends from a transcript that was never released. The key does not come all the way up, so a second attempt costs one more sequence number rather than a fresh session and a cold re-read.",
      },
      {
        name: "The D-pad",
        maker: "Nintendo; patented as “Multi-directional switch”, US 4,687,200, inventor Ichiro Shirai",
        year: "1982; patent filed 1985, granted 1987",
        url: "https://patents.google.com/patent/US4687200A/en",
        mechanic:
          "One cross-shaped rocker on an elastic seat, shaped so that pressing two protrusions at once still cannot close two contacts simultaneously.",
        taken:
          "The stops are mutually exclusive by construction. A turn resolves to exactly one point on the frontier, never a blend or a race between two models, so the dial reading always explains the bill.",
      },
      {
        name: "X-Y position indicator for a display system",
        maker: "Douglas Engelbart with Bill English, SRI; US 3,541,541",
        year: "1964 prototype, granted 1970",
        url: "https://patents.google.com/patent/US3541541A/en",
        mechanic:
          "Two wheels mounted at right angles, each rolling freely along one axis and skidding along the other, so one diagonal shove of the hand is read out as two independent values.",
        taken:
          "omni runs this transform backwards: two measured axes are folded into the one number a caller sets, and the fold is publishable only because just the frontier points survive it.",
      },
      {
        name: "The engine order telegraph",
        maker: "Chadburn's Ship Telegraph Company, Liverpool",
        year: "in service from the 19th century",
        url: "https://en.wikipedia.org/wiki/Engine_order_telegraph",
        mechanic:
          "Moving the bridge handle rings a bell at both ends, and the bell keeps ringing until the engineer below moves his own handle onto the same mark. That is the only confirmation the bridge ever gets.",
        taken:
          "The sequence number advances when the vendor's reply lands in the file, not when the prompt goes out — so a crashed CLI leaves no numbered turn and no phantom history to explain.",
      },
      {
        name: "The Anglepoise model 1227",
        maker: "George Carwardine",
        year: "1935",
        url: "https://en.wikipedia.org/wiki/Anglepoise_lamp",
        mechanic:
          "Constant-tension springs worked out from vehicle suspension hold the arm in equilibrium at any angle, so the lamp stays exactly where a hand left it with nothing clamped or locked.",
        taken:
          "The level is state on the object rather than an argument per call, so a mid-conversation vendor switch inherits the level you last set instead of a default nobody chose.",
      },
    ],
  },
]
