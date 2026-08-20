/** The litellm catalogue, narrowed to the vendors our three CLIs can reach. */

import { NextResponse } from "next/server"

import { models } from "../../../lib/litellm"

export const revalidate = 3600

export async function GET() {
  return NextResponse.json({ models: await models() })
}
