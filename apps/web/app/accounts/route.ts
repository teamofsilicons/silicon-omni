import { adminOrigin, authJson } from "@/lib/auth"

export const dynamic = "force-dynamic"

export function GET(): Response {
  return authJson({
    app_id: "omni",
    accounts_url: process.env.SILICON_ACCOUNTS_URL || "https://accounts.teamofsilicons.com",
    base_url: process.env.OMNI_API_ORIGIN || "https://backend.omni.teamofsilicons.com",
    admin_url: `${adminOrigin()}/admin`,
    docs: "https://backend.omni.teamofsilicons.com/docs",
    source: "https://github.com/teamofsilicons/silicon-omni",
  })
}
