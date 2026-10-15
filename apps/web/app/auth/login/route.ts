import { authError, startBrowser } from "@/lib/auth"

export const runtime = "nodejs"
export const dynamic = "force-dynamic"

export async function GET(request: Request): Promise<Response> {
  try { return await startBrowser(request) } catch (error) { return authError(error) }
}
