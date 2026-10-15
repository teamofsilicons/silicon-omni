import { fileURLToPath } from "node:url"
export default {
  outputFileTracingRoot: fileURLToPath(new URL(".", import.meta.url)),
  turbopack: { root: fileURLToPath(new URL(".", import.meta.url)) },
  async headers() {
    return [
      { source: "/:path*", headers: [
        { key: "X-Content-Type-Options", value: "nosniff" },
        { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
        { key: "X-Frame-Options", value: "DENY" },
      ] },
      ...["/choose.json", "/registry.json", "/models.json", "/categories"].map(source => ({
        source, headers: [
          { key: "Access-Control-Allow-Origin", value: "*" },
          { key: "Access-Control-Allow-Methods", value: "GET, OPTIONS" },
        ],
      })),
    ]
  },
}
