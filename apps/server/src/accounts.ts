import { parseRole, type Role } from "./roles";

// Users and devices are created here until Better Auth owns sign-in; there is deliberately
// no unauthenticated HTTP route for it.

export async function createUser(db: D1Database, user: { id: string; name: string }, now: number): Promise<void> {
  await db.prepare("INSERT INTO users (id, name, created_at) VALUES (?, ?, ?)").bind(user.id, user.name, now).run();
}

export async function addDevice(
  db: D1Database,
  device: { id: string; userId: string; name: string },
  now: number,
): Promise<void> {
  await db
    .prepare("INSERT INTO devices (id, user_id, name, created_at) VALUES (?, ?, ?, ?)")
    .bind(device.id, device.userId, device.name, now)
    .run();
}

export async function memberRole(db: D1Database, spaceId: string, userId: string): Promise<Role | null> {
  const row = await db
    .prepare("SELECT role FROM members WHERE space_id = ? AND user_id = ?")
    .bind(spaceId, userId)
    .first<{ role: string }>();
  return row ? parseRole(row.role) : null;
}

export type ClaimOutcome = "created" | "already_owner" | "taken";

/** Registers a new space with the caller as its owner. The first claim on an id wins. */
export async function claimSpace(db: D1Database, spaceId: string, userId: string, now: number): Promise<ClaimOutcome> {
  const result = await db
    .prepare(
      `INSERT INTO members (space_id, user_id, role, joined_at)
       SELECT ?1, ?2, 'Owner', ?3
       WHERE NOT EXISTS (SELECT 1 FROM members WHERE space_id = ?1)`,
    )
    .bind(spaceId, userId, now)
    .run();
  if (result.meta.changes === 1) return "created";
  return (await memberRole(db, spaceId, userId)) === "Owner" ? "already_owner" : "taken";
}
