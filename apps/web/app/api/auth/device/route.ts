import { authError, startDevice } from "@/lib/auth"

export const runtime = "nodejs"
export const dynamic = "force-dynamic"

export async function POST(request: Request): Promise<Response> {
  try { return await startDevice(request) } catch (error) { return authError(error) }
}
