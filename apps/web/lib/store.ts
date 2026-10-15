import { GetObjectCommand, PutObjectCommand, S3Client } from "@aws-sdk/client-s3"
import { awsCredentialsProvider } from "@vercel/oidc-aws-credentials-provider"
import bundled from "../models.json"
import { validateCatalog, type Catalog } from "./catalog"
const KEY = "catalog.json"
export class CatalogConflict extends Error {}
function client() {
  return new S3Client({
    region: process.env.OMNI_AWS_REGION || "us-west-1",
    ...(process.env.AWS_ROLE_ARN ? { credentials: awsCredentialsProvider({ roleArn: process.env.AWS_ROLE_ARN }) } : {}),
  })
}
export async function readCatalog(): Promise<{ catalog: Catalog; revision: string; writable: boolean }> {
  const bucket = process.env.OMNI_CATALOG_BUCKET
  if (!bucket) {
    if (process.env.VERCEL_ENV === "production") throw new Error("Catalog storage is not configured.")
    return { catalog: validateCatalog(bundled), revision: "bundled", writable: false }
  }
  // A storage failure must not silently restore an older bundled catalog.
  const result = await client().send(new GetObjectCommand({ Bucket: bucket, Key: KEY }))
  if (!result.Body || !result.ETag) throw new Error("Catalog storage returned an empty object.")
  return { catalog: validateCatalog(JSON.parse(await result.Body.transformToString())), revision: result.ETag, writable: true }
}
export async function writeCatalog(raw: unknown, revision: string, actor: string) {
  const bucket = process.env.OMNI_CATALOG_BUCKET
  if (!bucket) throw new Error("Catalog storage is not configured. Local seed data is read-only.")
  if (!/^"[a-f0-9-]+"$/.test(revision)) throw new Error("A current catalog revision is required.")
  const catalog: Catalog = { ...validateCatalog(raw), updated_at: new Date().toISOString(), updated_by: actor }
  try {
    const result = await client().send(new PutObjectCommand({
      Bucket: bucket, Key: KEY, Body: JSON.stringify(catalog), ContentType: "application/json", IfMatch: revision,
    }))
    return { catalog, revision: result.ETag!, writable: true }
  } catch (error) {
    const code = (error as { $metadata?: { httpStatusCode?: number } }).$metadata?.httpStatusCode
    if (code === 412 || code === 409) throw new CatalogConflict("Someone saved a newer catalog. Reload before publishing your changes.")
    throw error
  }
}
