import { NextRequest, NextResponse } from "next/server"
export function middleware(request: NextRequest) {
  const url = request.nextUrl.clone()
  if (url.pathname === "/" && url.hostname === "admin.omni.teamofsilicons.com") {
    url.pathname = "/admin"
    return NextResponse.rewrite(url)
  }
  if (url.pathname === "/" && url.hostname === "backend.omni.teamofsilicons.com") {
    url.pathname = "/registry.json"
    return NextResponse.rewrite(url)
  }
  if (url.pathname === "/models" && (url.hostname === "backend.omni.teamofsilicons.com" || request.headers.get("accept")?.includes("application/json"))) {
    url.pathname = "/models.json"
    return NextResponse.rewrite(url)
  }
  return NextResponse.next()
}
export const config = { matcher: ["/", "/models"] }
