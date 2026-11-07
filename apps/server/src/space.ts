import { DurableObject } from "cloudflare:workers";
import { parseClientMessage, parseSince, type ServerMessage, type StoredChange } from "./protocol";
import { canPush, parseRole, type Role } from "./roles";

/** Who is on the other end of a socket. Set by the Worker after it checked auth and membership. */
export interface SocketIdentity {
  userId: string;
  deviceId: string;
  role: Role;
}

/** Internal header the Worker uses to hand the checked identity to the Durable Object. */
export const IDENTITY_HEADER = "X-Space-Identity";

const BACKLOG_PAGE = 500;

interface ChangeRow extends Record<string, SqlStorageValue> {
  seq: number;
  change_id: string;
  device: string;
  hlc: string;
  payload: string;
}

/**
 * One instance per space. Holds the space's append-only change log and relays new changes
 * to every connected device. Uses the WebSocket hibernation API, so idle spaces cost nothing.
 */
export class SpaceObject extends DurableObject<Env> {
  private readonly sql: SqlStorage;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    // AUTOINCREMENT so a seq is never reused, even if old rows are purged one day.
    this.sql.exec(`CREATE TABLE IF NOT EXISTS changes (
      seq       INTEGER PRIMARY KEY AUTOINCREMENT,
      change_id TEXT NOT NULL UNIQUE,
      device    TEXT NOT NULL,
      hlc       TEXT NOT NULL,
      payload   TEXT NOT NULL
    )`);
  }

  override async fetch(request: Request): Promise<Response> {
    const identity = parseIdentity(request.headers.get(IDENTITY_HEADER));
    if (!identity) return new Response("missing identity", { status: 400 });
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("expected a WebSocket upgrade", { status: 426 });
    }
    const since = parseSince(new URL(request.url).searchParams.get("since"));
    if (since === null) return new Response("bad since", { status: 400 });

    const { 0: client, 1: server } = new WebSocketPair();
    this.ctx.acceptWebSocket(server);
    server.serializeAttachment(identity);
    this.sendBacklog(server, since);
    return new Response(null, { status: 101, webSocket: client });
  }

  override async webSocketMessage(ws: WebSocket, raw: string | ArrayBuffer): Promise<void> {
    const identity = ws.deserializeAttachment() as SocketIdentity;
    const parsed = parseClientMessage(raw);
    if (!parsed.ok) {
      send(ws, { type: "error", code: "bad_message", message: parsed.reason });
      return;
    }
    if (!canPush(identity.role)) {
      send(ws, { type: "error", code: "read_only", message: "viewers cannot change this space" });
      return;
    }

    const stored: StoredChange[] = [];
    const duplicates: string[] = [];
    this.ctx.storage.transactionSync(() => {
      for (const change of parsed.value.changes) {
        // Checked before inserting because an ignored conflict still advances AUTOINCREMENT,
        // which would leave gaps in seq.
        const exists = this.sql.exec("SELECT 1 FROM changes WHERE change_id = ?", change.change_id).toArray();
        if (exists.length > 0) {
          duplicates.push(change.change_id);
          continue;
        }
        const { seq } = this.sql
          .exec<{ seq: number }>(
            "INSERT INTO changes (change_id, device, hlc, payload) VALUES (?, ?, ?, ?) RETURNING seq",
            change.change_id,
            identity.deviceId,
            change.hlc,
            JSON.stringify(change.payload),
          )
          .one();
        stored.push({ ...change, seq, device: identity.deviceId });
      }
    });

    send(ws, { type: "ack", seq: this.headSeq(), duplicates });
    if (stored.length === 0) return;
    const broadcast: ServerMessage = { type: "changes", changes: stored };
    for (const peer of this.ctx.getWebSockets()) {
      if (peer !== ws) send(peer, broadcast);
    }
  }

  override async webSocketClose(ws: WebSocket, code: number, reason: string): Promise<void> {
    try {
      ws.close(code, reason);
    } catch {
      // Already closed by the runtime.
    }
  }

  private sendBacklog(ws: WebSocket, since: number): void {
    let cursor = since;
    for (;;) {
      const rows = this.sql
        .exec<ChangeRow>(
          "SELECT seq, change_id, device, hlc, payload FROM changes WHERE seq > ? ORDER BY seq LIMIT ?",
          cursor,
          BACKLOG_PAGE,
        )
        .toArray();
      if (rows.length === 0) break;
      send(ws, { type: "changes", changes: rows.map(toStoredChange) });
      cursor = rows[rows.length - 1]!.seq;
      if (rows.length < BACKLOG_PAGE) break;
    }
    send(ws, { type: "synced", seq: this.headSeq() });
  }

  private headSeq(): number {
    return this.sql.exec<{ head: number }>("SELECT COALESCE(MAX(seq), 0) AS head FROM changes").one().head;
  }
}

function toStoredChange(row: ChangeRow): StoredChange {
  return {
    seq: row.seq,
    change_id: row.change_id,
    device: row.device,
    hlc: row.hlc,
    payload: JSON.parse(row.payload) as unknown,
  };
}

function send(ws: WebSocket, message: ServerMessage): void {
  ws.send(JSON.stringify(message));
}

function parseIdentity(header: string | null): SocketIdentity | null {
  if (!header) return null;
  try {
    const value = JSON.parse(header) as Record<string, unknown>;
    const role = parseRole(value.role);
    if (typeof value.userId !== "string" || typeof value.deviceId !== "string" || !role) return null;
    return { userId: value.userId, deviceId: value.deviceId, role };
  } catch {
    return null;
  }
}
