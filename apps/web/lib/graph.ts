import { unstable_noStore as noStore } from "next/cache"
import type { Entry, Key } from "./choose"
import { readCatalog } from "./store"
export const SOURCE = "https://backend.omni.teamofsilicons.com/models.json"
export const VIEW = SOURCE
export const EDIT = "https://admin.omni.teamofsilicons.com"
export interface Bencher { label: string; short: string; url: string }
export interface Graph {
  models: Entry[]
  benches: Record<string, Bencher>
  keys: Record<string, Key>
  source: Record<string, string>
  caveats: string[]
  fresh: boolean
}
export async function graph(): Promise<Graph> {
  noStore()
  const { catalog, writable } = await readCatalog()
  return { models: catalog.models, benches: catalog.benches, keys: catalog.keys,
    source: catalog.source, caveats: catalog.caveats, fresh: writable }
}
