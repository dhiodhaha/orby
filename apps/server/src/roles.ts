/** A member's role in a space. Exactly one Owner per space (enforced in D1). */
export type Role = "Owner" | "Editor" | "Viewer";

/** Roles an invite can grant. Ownership is never handed out by link. */
export type InviteRole = Exclude<Role, "Owner">;

export function parseRole(value: unknown): Role | null {
  return value === "Owner" || value === "Editor" || value === "Viewer" ? value : null;
}

export function parseInviteRole(value: unknown): InviteRole | null {
  return value === "Editor" || value === "Viewer" ? value : null;
}

/** Editors add, edit and finish; viewers only see. */
export function canPush(role: Role): boolean {
  return role !== "Viewer";
}

/** Only the owner invites or removes people. */
export function canInvite(role: Role): boolean {
  return role === "Owner";
}
