import { claimSpace, memberRole } from "./accounts";
import { authenticate, type Principal } from "./auth";
import { acceptInvite, createInvite } from "./invites";
import { parseSince } from "./protocol";
import { canInvite, parseInviteRole } from "./roles";
import { IDENTITY_HEADER, type SocketIdentity } from "./space";

type Route =
  | { kind: "sync"; spaceId: string }
  | { kind: "claim_space"; spaceId: string }
  | { kind: "create_invite"; spaceId: string }
  | { kind: "accept_invite"; token: string };

const ID = "([A-Za-z0-9_-]{1,64})";
const ROUTES: { method: string; pattern: RegExp; build: (param: string) => Route }[] = [
  { method: "GET", pattern: new RegExp(`^/spaces/${ID}/sync$`), build: (spaceId) => ({ kind: "sync", spaceId }) },
  { method: "PUT", pattern: new RegExp(`^/spaces/${ID}$`), build: (spaceId) => ({ kind: "claim_space", spaceId }) },
  {
    method: "POST",
    pattern: new RegExp(`^/spaces/${ID}/invites$`),
    build: (spaceId) => ({ kind: "create_invite", spaceId }),
  },
  {
    method: "POST",
    pattern: /^\/invites\/([A-Za-z0-9_-]{1,128})\/accept$/,
    build: (token) => ({ kind: "accept_invite", token }),
  },
];

function matchRoute(method: string, pathname: string): Route | null {
  for (const route of ROUTES) {
    const match = route.method === method ? route.pattern.exec(pathname) : null;
    if (match?.[1]) return route.build(match[1]);
  }
  return null;
}

/** The whole HTTP surface. `now` is injected so invite expiry is testable; index.ts passes the real clock. */
export async function handleRequest(request: Request, env: Env, now: number): Promise<Response> {
  const url = new URL(request.url);
  const route = matchRoute(request.method, url.pathname);
  if (!route) return error(404, "not_found");

  const auth = await authenticate(request, env);
  if (!auth.ok) return error(401, auth.reason);
  const principal = auth.principal;

  switch (route.kind) {
    case "sync":
      return openSync(request, env, principal, route.spaceId, url);
    case "claim_space":
      return claim(env, principal, route.spaceId, now);
    case "create_invite":
      return invite(request, env, principal, route.spaceId, now);
    case "accept_invite":
      return accept(env, principal, route.token, now);
  }
}

async function openSync(request: Request, env: Env, principal: Principal, spaceId: string, url: URL) {
  if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") return error(426, "websocket_required");
  const since = parseSince(url.searchParams.get("since"));
  if (since === null) return error(400, "bad_since");
  const role = await memberRole(env.DB, spaceId, principal.userId);
  if (!role) return error(403, "not_a_member");

  // The role is captured at connect time; a role change takes effect on the next connect.
  const identity: SocketIdentity = { userId: principal.userId, deviceId: principal.deviceId, role };
  const stub = env.SPACE.get(env.SPACE.idFromName(spaceId));
  return stub.fetch(`https://space.internal/sync?since=${since}`, {
    headers: { Upgrade: "websocket", [IDENTITY_HEADER]: JSON.stringify(identity) },
  });
}

async function claim(env: Env, principal: Principal, spaceId: string, now: number) {
  const outcome = await claimSpace(env.DB, spaceId, principal.userId, now);
  switch (outcome) {
    case "created":
      return json(201, { spaceId, role: "Owner" });
    case "already_owner":
      return json(200, { spaceId, role: "Owner" });
    case "taken":
      return error(409, "space_taken");
  }
}

async function invite(request: Request, env: Env, principal: Principal, spaceId: string, now: number) {
  const role = await memberRole(env.DB, spaceId, principal.userId);
  if (!role) return error(403, "not_a_member");
  if (!canInvite(role)) return error(403, "owner_only");

  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return error(400, "bad_json");
  }
  const inviteRole = parseInviteRole((body as { role?: unknown } | null)?.role);
  if (!inviteRole) return error(400, "bad_role");

  const created = await createInvite(env.DB, { spaceId, role: inviteRole, createdBy: principal.userId }, now);
  return json(201, created);
}

async function accept(env: Env, principal: Principal, token: string, now: number) {
  const outcome = await acceptInvite(env.DB, token, principal.userId, now);
  switch (outcome.kind) {
    case "joined":
      return json(200, { spaceId: outcome.spaceId, role: outcome.role });
    case "already_member":
      return json(409, { error: "already_member", spaceId: outcome.spaceId, role: outcome.role });
    case "not_found":
      return error(404, "invite_not_found");
    case "used":
      return error(409, "invite_used");
    case "expired":
      return error(410, "invite_expired");
  }
}

function json(status: number, body: unknown): Response {
  return Response.json(body, { status });
}

function error(status: number, code: string): Response {
  return json(status, { error: code });
}
