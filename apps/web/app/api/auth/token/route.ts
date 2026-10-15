import { authError, loginWithGrant } from "@/lib/auth"

export const runtime = "nodejs"
export const dynamic = "force-dynamic"

export async function POST(request: Request): Promise<Response> {
  try { return await loginWithGrant(request, "device") } catch (error) { return authError(error) }
}
