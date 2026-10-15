import { authError, completeBrowser } from "@/lib/auth"

export const runtime = "nodejs"
export const dynamic = "force-dynamic"

export async function GET(request: Request): Promise<Response> {
  try { return await completeBrowser(request) } catch (error) { return authError(error) }
}
