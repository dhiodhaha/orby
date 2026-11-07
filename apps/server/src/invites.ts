import { base64url } from "./encoding";
import { parseInviteRole, parseRole, type InviteRole, type Role } from "./roles";

export const INVITE_TTL_MS = 7 * 24 * 60 * 60 * 1000;

export interface Invite {
  /** The secret that goes into the invite link. Only its hash is stored. */
  token: string;
  spaceId: string;
  role: InviteRole;
  expiresAt: number;
}

export async function createInvite(
  db: D1Database,
  input: { spaceId: string; role: InviteRole; createdBy: string },
  now: number,
): Promise<Invite> {
  const token = base64url(crypto.getRandomValues(new Uint8Array(32)));
  const expiresAt = now + INVITE_TTL_MS;
  await db
    .prepare(
      "INSERT INTO invites (token_hash, space_id, role, created_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(await hashToken(token), input.spaceId, input.role, input.createdBy, now, expiresAt)
    .run();
  return { token, spaceId: input.spaceId, role: input.role, expiresAt };
}

export interface InviteRecord {
  spaceId: string;
  role: InviteRole;
  expiresAt: number;
  acceptedAt: number | null;
}

export type InviteState =
  | { kind: "open"; spaceId: string; role: InviteRole }
  | { kind: "used" }
  | { kind: "expired" };

/** An invite is open strictly before `expiresAt`. A used invite stays used even after it expires. */
export function inviteState(invite: InviteRecord, now: number): InviteState {
  if (invite.acceptedAt !== null) return { kind: "used" };
  if (now >= invite.expiresAt) return { kind: "expired" };
  return { kind: "open", spaceId: invite.spaceId, role: invite.role };
}

export type AcceptOutcome =
  | { kind: "joined"; spaceId: string; role: InviteRole }
  | { kind: "already_member"; spaceId: string; role: Role }
  | { kind: "not_found" }
  | { kind: "used" }
  | { kind: "expired" };

/** Joins the user to the invite's space. An existing member does not use up the invite. */
export async function acceptInvite(db: D1Database, token: string, userId: string, now: number): Promise<AcceptOutcome> {
  const tokenHash = await hashToken(token);
  const row = await db
    .prepare("SELECT space_id, role, expires_at, accepted_at FROM invites WHERE token_hash = ?")
    .bind(tokenHash)
    .first<{ space_id: string; role: string; expires_at: number; accepted_at: number | null }>();
  const role = row ? parseInviteRole(row.role) : null;
  if (!row || !role) return { kind: "not_found" };

  const state = inviteState(
    { spaceId: row.space_id, role, expiresAt: row.expires_at, acceptedAt: row.accepted_at },
    now,
  );
  if (state.kind !== "open") return state;

  const existing = await db
    .prepare("SELECT role FROM members WHERE space_id = ? AND user_id = ?")
    .bind(state.spaceId, userId)
    .first<{ role: string }>();
  const existingRole = existing ? parseRole(existing.role) : null;
  if (existingRole) return { kind: "already_member", spaceId: state.spaceId, role: existingRole };

  // The guarded UPDATE makes two concurrent accepts race safely: only one flips accepted_at,
  // and the member row is copied only from the invite this user just claimed.
  const [claimed] = await db.batch([
    db
      .prepare("UPDATE invites SET accepted_by = ?, accepted_at = ? WHERE token_hash = ? AND accepted_at IS NULL")
      .bind(userId, now, tokenHash),
    db
      .prepare(
        `INSERT OR IGNORE INTO members (space_id, user_id, role, joined_at)
         SELECT space_id, accepted_by, role, accepted_at FROM invites
         WHERE token_hash = ? AND accepted_by = ? AND accepted_at = ?`,
      )
      .bind(tokenHash, userId, now),
  ]);
  if (!claimed || claimed.meta.changes !== 1) return { kind: "used" };
  return { kind: "joined", spaceId: state.spaceId, role: state.role };
}

async function hashToken(token: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}
