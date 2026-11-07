# app-server

The sync relay for shared spaces (ORB_SPEC section 9, DESIGN.md "Tasks panel and sharing (v6.2)").
A Cloudflare Worker with one Durable Object per space. It stores and relays each space's change log;
it never merges. Per-field last-write-wins happens in `crates/sync`.

- **D1 (`DB`)**: `users`, `devices`, `members(space_id, user_id, role)`, `invites`. Schema in `migrations/0001_init.sql`.
  A partial unique index allows exactly one Owner per space. Invites store only a SHA-256 of the token.
- **Durable Object (`SPACE`, class `SpaceObject`, SQLite storage)**: table
  `changes(seq INTEGER PRIMARY KEY AUTOINCREMENT, change_id UNIQUE, device, hlc, payload JSON)`.
  WebSockets use the hibernation API; each socket's identity and role ride in its attachment.
- **Secret `DEVICE_TOKEN_SECRET`**: HMAC-SHA256 key for device tokens.

## HTTP API

Every route needs `Authorization: Bearer <device token>`. Errors are `{ "error": "<code>" }`.

| Route | Who | Result |
| --- | --- | --- |
| `PUT /spaces/:id` | any user | 201 caller becomes Owner; 200 if already theirs; 409 `space_taken` |
| `GET /spaces/:id/sync?since=N` (WebSocket upgrade) | any member | 101; 401 bad token; 403 `not_a_member`; 426 not an upgrade; 400 `bad_since` |
| `POST /spaces/:id/invites` `{ "role": "Editor" \| "Viewer" }` | Owner only | 201 `{ token, spaceId, role, expiresAt }`, valid 7 days; 403 `owner_only`; 400 `bad_role` |
| `POST /invites/:token/accept` | any user | 200 `{ spaceId, role }`; 404 `invite_not_found`; 409 `invite_used`; 410 `invite_expired`; 409 `already_member` (invite not consumed) |

## Sync protocol (`src/protocol.ts`)

Client to server: `{ "type": "push", "changes": [{ "change_id", "hlc", "payload" }] }` (at most 500 per push).
The server stamps `device` from the token, never from the client.

Server to client:
- `{ "type": "changes", "changes": [StoredChange] }`: backlog pages after `since` on connect, and broadcasts of other devices' pushes. `StoredChange = { seq, change_id, device, hlc, payload }`.
- `{ "type": "synced", "seq" }`: end of the backlog; `seq` is the log head.
- `{ "type": "ack", "seq", "duplicates" }`: reply to a push; `seq` is the head after it, `duplicates` lists change_ids already in the log (ignored, first copy kept). The pusher does not get its own changes echoed.
- `{ "type": "error", "code": "bad_message" | "read_only", "message" }`: a viewer's push is refused and nothing is stored; the socket stays open.

## Modules

- `src/auth.ts`: `issueDeviceToken(secret, principal)`, `verifyDeviceToken(secret, token)`, `authenticate(request, env)`. The only place that knows the token format; swap it for Better Auth later.
- `src/accounts.ts`: `createUser`, `addDevice` (the seam until Better Auth owns sign-in), `memberRole`, `claimSpace`.
- `src/invites.ts`: `createInvite`, `acceptInvite`, pure `inviteState(invite, now)`, `INVITE_TTL_MS`.
- `src/roles.ts`: `Role`, `InviteRole`, `canPush`, `canInvite`.
- `src/space.ts`: `SpaceObject` (Durable Object).
- `src/router.ts`: `handleRequest(request, env, now)`. `src/index.ts` is the only place that reads `Date.now()`.

## Verification

```powershell
cd C:\Users\dopamine\dev\clippy\apps\server; npm install; npm test; npx tsc --noEmit
```

Last run: 3 test files, **21 tests passed**, `tsc --noEmit` clean. Tests run in workerd through
`@cloudflare/vitest-plugin` (Miniflare) with real D1 and Durable Object bindings. They cover push and
broadcast between sockets, backlog since N, duplicate change_id ignored, viewer push rejected,
relay and roles after hibernation, invite accept, reuse, expiry at exactly 7 days, owner-only invites,
and token tampering.

`npm run types` regenerates `worker-configuration.d.ts` after a `wrangler.jsonc` change.
Never deployed; `database_id` in `wrangler.jsonc` is a placeholder.
