import { authError, authJson, getSession } from "@/lib/auth"

export const runtime = "nodejs"
export const dynamic = "force-dynamic"

export async function GET(request: Request): Promise<Response> {
  try { return await authJson(await getSession(request)) } catch (error) { return authError(error) }
}
