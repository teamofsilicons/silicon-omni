import { fileURLToPath } from "node:url"

/** @type {import('next').NextConfig} */
export default {
  // Pin the root: there are lockfiles above this directory that are none of our business.
  outputFileTracingRoot: fileURLToPath(new URL(".", import.meta.url)),
  turbopack: { root: fileURLToPath(new URL(".", import.meta.url)) },
  // The dial is read by omni from anywhere, so the endpoint has to be open.
  async headers() {
    return [
      {
        source: "/api/:path*",
        headers: [
          { key: "Access-Control-Allow-Origin", value: "*" },
          { key: "Access-Control-Allow-Headers", value: "content-type, x-omni-token" },
          { key: "Access-Control-Allow-Methods", value: "GET, POST, DELETE, OPTIONS" },
        ],
      },
    ]
  },
}
