import { exports } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { issueDeviceToken, verifyDeviceToken } from "../src/auth";
import { base64url } from "../src/encoding";
import { inviteState } from "../src/invites";
import { MAX_CHANGES_PER_PUSH, parseClientMessage, parseSince } from "../src/protocol";

describe("device tokens", () => {
  const principal = { userId: "user-1", deviceId: "device-1" };

  it("round-trips the principal", async () => {
    const token = await issueDeviceToken("secret", principal);
    expect(await verifyDeviceToken("secret", token)).toEqual(principal);
  });

  it("rejects a token signed with another secret or with edited claims", async () => {
    const token = await issueDeviceToken("secret", principal);
    expect(await verifyDeviceToken("other", token)).toBeNull();

    const [version, , signature] = token.split(".");
    const forged = base64url(new TextEncoder().encode(JSON.stringify({ sub: "user-2", dev: "device-1" })));
    expect(await verifyDeviceToken("secret", `${version}.${forged}.${signature}`)).toBeNull();
    expect(await verifyDeviceToken("secret", "garbage")).toBeNull();
  });
});

describe("invite state", () => {
  const open = { spaceId: "s", role: "Editor" as const, expiresAt: 1000, acceptedAt: null };

  it("is open before expiry, expired at it, and used once accepted", () => {
    expect(inviteState(open, 999)).toEqual({ kind: "open", spaceId: "s", role: "Editor" });
    expect(inviteState(open, 1000)).toEqual({ kind: "expired" });
    expect(inviteState({ ...open, acceptedAt: 10 }, 5000)).toEqual({ kind: "used" });
  });
});

describe("protocol parsing", () => {
  it("accepts a push and keeps any JSON payload", () => {
    const raw = JSON.stringify({ type: "push", changes: [{ change_id: "c1", hlc: "h", payload: null }] });
    expect(parseClientMessage(raw)).toEqual({
      ok: true,
      value: { type: "push", changes: [{ change_id: "c1", hlc: "h", payload: null }] },
    });
  });

  it("rejects unknown types, binary frames, missing payloads and oversized pushes", () => {
    expect(parseClientMessage(JSON.stringify({ type: "pull" })).ok).toBe(false);
    expect(parseClientMessage(new ArrayBuffer(4)).ok).toBe(false);
    expect(parseClientMessage(JSON.stringify({ type: "push", changes: [{ change_id: "c", hlc: "h" }] })).ok).toBe(
      false,
    );
    const many = Array.from({ length: MAX_CHANGES_PER_PUSH + 1 }, (_, i) => ({ change_id: `c${i}`, hlc: "h", payload: 1 }));
    expect(parseClientMessage(JSON.stringify({ type: "push", changes: many })).ok).toBe(false);
  });

  it("reads since as a non-negative integer, defaulting to 0", () => {
    expect(parseSince(null)).toBe(0);
    expect(parseSince("42")).toBe(42);
    expect(parseSince("-1")).toBeNull();
    expect(parseSince("1.5")).toBeNull();
  });
});

describe("worker entry", () => {
  it("serves the router through the default export", async () => {
    const missing = await exports.default.fetch("https://app.test/nope");
    expect(missing.status).toBe(404);
    const unauthenticated = await exports.default.fetch("https://app.test/invites/abc/accept", { method: "POST" });
    expect(unauthenticated.status).toBe(401);
  });
});
