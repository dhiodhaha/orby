// Wire format of /spaces/:id/sync. The server never looks inside `payload`;
// merging is the client's job (crates/sync).

/** A change as a device sends it. `device` is not trusted from the client; the server stamps it. */
export interface ClientChange {
  change_id: string;
  hlc: string;
  payload: unknown;
}

/** A change as it sits in the space's log. `seq` is assigned by the server and only grows. */
export interface StoredChange extends ClientChange {
  seq: number;
  device: string;
}

export type ClientMessage = { type: "push"; changes: ClientChange[] };

export type ErrorCode = "bad_message" | "read_only";

export type ServerMessage =
  /** Log entries: the backlog on connect (in pages), or a broadcast of another device's push. */
  | { type: "changes"; changes: StoredChange[] }
  /** Sent once after the backlog: the client has everything up to `seq`. */
  | { type: "synced"; seq: number }
  /** Reply to a push. `seq` is the log head after it; `duplicates` were already in the log and were ignored. */
  | { type: "ack"; seq: number; duplicates: string[] }
  | { type: "error"; code: ErrorCode; message: string };

export const MAX_CHANGES_PER_PUSH = 500;
const MAX_ID_LENGTH = 128;

export type Parsed<T> = { ok: true; value: T } | { ok: false; reason: string };

export function parseClientMessage(raw: string | ArrayBuffer): Parsed<ClientMessage> {
  if (typeof raw !== "string") return { ok: false, reason: "messages must be text" };
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return { ok: false, reason: "not JSON" };
  }
  if (!isRecord(data) || data.type !== "push") return { ok: false, reason: "unknown message type" };
  const { changes } = data;
  if (!Array.isArray(changes)) return { ok: false, reason: "changes must be an array" };
  if (changes.length > MAX_CHANGES_PER_PUSH) {
    return { ok: false, reason: `at most ${MAX_CHANGES_PER_PUSH} changes per push` };
  }

  const parsed: ClientChange[] = [];
  for (const change of changes) {
    if (!isRecord(change)) return { ok: false, reason: "each change must be an object" };
    const { change_id, hlc } = change;
    if (!isId(change_id)) return { ok: false, reason: "change_id must be a non-empty string" };
    if (!isId(hlc)) return { ok: false, reason: "hlc must be a non-empty string" };
    if (!("payload" in change)) return { ok: false, reason: "payload is required" };
    parsed.push({ change_id, hlc, payload: change.payload });
  }
  return { ok: true, value: { type: "push", changes: parsed } };
}

/** `?since=N`: the last seq the device has seen. Missing means 0 (send everything). */
export function parseSince(value: string | null): number | null {
  if (value === null || value === "") return 0;
  if (!/^\d{1,15}$/.test(value)) return null;
  return Number(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isId(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= MAX_ID_LENGTH;
}
