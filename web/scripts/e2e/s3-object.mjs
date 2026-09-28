// Minimal signed GET/HEAD for this runner's own objects; credentials/URLs stay in memory.
/* global fetch, URL */
import { createHash, createHmac } from "node:crypto";
import assert from "node:assert/strict";
export async function objectStatus(origin, bucket, objectKey, access, secret) {
  const url = new URL(origin);
  assert.equal(url.protocol, "http:");
  assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(url.hostname));
  assert.match(bucket, /^gb-fault-[a-f0-9-]+$/);
  const encode = (v) =>
    encodeURIComponent(v).replace(
      /[!'()*]/g,
      (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`,
    );
  url.pathname = `/${bucket}/${objectKey.split("/").map(encode).join("/")}`;
  const hash = (v) => createHash("sha256").update(v).digest("hex");
  const hmac = (k, v) => createHmac("sha256", k).update(v).digest();
  const date = new Date().toISOString().replace(/[-:]|\.\d{3}/g, "");
  const scope = `${date.slice(0, 8)}/us-east-1/s3/aws4_request`;
  const headers = { "x-amz-content-sha256": hash(""), "x-amz-date": date };
  const canonical = `host:${url.host}\nx-amz-content-sha256:${headers["x-amz-content-sha256"]}\nx-amz-date:${date}\n`;
  const signed = "host;x-amz-content-sha256;x-amz-date";
  const request = `GET\n${url.pathname}\n\n${canonical}\n${signed}\n${hash("")}`;
  const key = hmac(
    hmac(hmac(hmac(`AWS4${secret}`, date.slice(0, 8)), "us-east-1"), "s3"),
    "aws4_request",
  );
  const signature = createHmac("sha256", key)
    .update(`AWS4-HMAC-SHA256\n${date}\n${scope}\n${hash(request)}`)
    .digest("hex");
  headers.Authorization = `AWS4-HMAC-SHA256 Credential=${access}/${scope}, SignedHeaders=${signed}, Signature=${signature}`;
  const response = await fetch(url, { headers });
  await response.arrayBuffer();
  return response.status;
}
