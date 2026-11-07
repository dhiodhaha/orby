// Device authentication. Everything about how a device proves who it is lives here,
// so Better Auth can replace this module later without touching routes or the Durable Object.

import { base64url, fromBase64url } from "./encoding";

export interface Principal {
  userId: string;
  deviceId: string;
}

export type AuthFailure = "missing_token" | "invalid_token" | "unknown_device";

export type AuthResult = { ok: true; principal: Principal } | { ok: false; reason: AuthFailure };

const TOKEN_VERSION = "v1";
const encoder = new TextEncoder();

/** Token shape: `v1.<base64url claims JSON>.<base64url HMAC-SHA256 of "v1.<claims>">`. */
export async function issueDeviceToken(secret: string, principal: Principal): Promise<string> {
  const claims = base64url(encoder.encode(JSON.stringify({ sub: principal.userId, dev: principal.deviceId })));
  const signed = `${TOKEN_VERSION}.${claims}`;
  const signature = await crypto.subtle.sign("HMAC", await hmacKey(secret), encoder.encode(signed));
  return `${signed}.${base64url(new Uint8Array(signature))}`;
}

/** Checks the signature only. Returns the claimed principal, or null for any malformed or forged token. */
export async function verifyDeviceToken(secret: string, token: string): Promise<Principal | null> {
  const parts = token.split(".");
  if (parts.length !== 3 || parts[0] !== TOKEN_VERSION) return null;
  const [version, claims, signature] = parts as [string, string, string];
  const signatureBytes = fromBase64url(signature);
  const claimsBytes = fromBase64url(claims);
  if (!signatureBytes || !claimsBytes) return null;

  // subtle.verify compares in constant time.
  const valid = await crypto.subtle.verify(
    "HMAC",
    await hmacKey(secret),
    signatureBytes,
    encoder.encode(`${version}.${claims}`),
  );
  if (!valid) return null;

  try {
    const parsed: unknown = JSON.parse(new TextDecoder().decode(claimsBytes));
    if (typeof parsed !== "object" || parsed === null) return null;
    const { sub, dev } = parsed as Record<string, unknown>;
    if (typeof sub !== "string" || typeof dev !== "string" || !sub || !dev) return null;
    return { userId: sub, deviceId: dev };
  } catch {
    return null;
  }
}

/**
 * Reads `Authorization: Bearer <token>`, verifies it, and checks the device is still registered
 * to that user. Deleting a device row revokes its token.
 */
export async function authenticate(request: Request, env: Env): Promise<AuthResult> {
  const header = request.headers.get("Authorization");
  const match = header?.match(/^Bearer\s+(\S+)$/i);
  if (!match?.[1]) return { ok: false, reason: "missing_token" };

  const principal = await verifyDeviceToken(env.DEVICE_TOKEN_SECRET, match[1]);
  if (!principal) return { ok: false, reason: "invalid_token" };

  const device = await env.DB.prepare("SELECT 1 AS found FROM devices WHERE id = ? AND user_id = ?")
    .bind(principal.deviceId, principal.userId)
    .first<{ found: number }>();
  if (!device) return { ok: false, reason: "unknown_device" };

  return { ok: true, principal };
}

async function hmacKey(secret: string): Promise<CryptoKey> {
  return crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
    "verify",
  ]);
}
