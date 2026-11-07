import { env } from "cloudflare:workers";
import { addDevice, createUser } from "../src/accounts";
import { issueDeviceToken } from "../src/auth";
import type { ClientChange, ServerMessage, StoredChange } from "../src/protocol";
import { handleRequest } from "../src/router";

/** A fixed "now" for every test: 2026-11-06 09:00 UTC. */
export const NOW = Date.UTC(2026, 10, 6, 9, 0, 0);

export interface TestDevice {
  userId: string;
  deviceId: string;
  token: string;
}

export function uid(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`;
}

/** A new user with one registered device and its bearer token. */
export async function newUser(name: string): Promise<TestDevice> {
  const userId = uid("user");
  await createUser(env.DB, { id: userId, name }, NOW);
  return newDevice(userId);
}

export async function newDevice(userId: string): Promise<TestDevice> {
  const deviceId = uid("device");
  await addDevice(env.DB, { id: deviceId, userId, name: "PC" }, NOW);
  return { userId, deviceId, token: await issueDeviceToken(env.DEVICE_TOKEN_SECRET, { userId, deviceId }) };
}

export function call(
  method: string,
  path: string,
  options: { token?: string; body?: unknown; headers?: Record<string, string>; now?: number } = {},
): Promise<Response> {
  const headers = new Headers(options.headers);
  if (options.token) headers.set("Authorization", `Bearer ${options.token}`);
  const init: RequestInit = { method, headers };
  if (options.body !== undefined) {
    headers.set("Content-Type", "application/json");
    init.body = JSON.stringify(options.body);
  }
  return handleRequest(new Request(`https://app.test${path}`, init), env, options.now ?? NOW);
}

/** Creates a space owned by `owner`. */
export async function newSpace(owner: TestDevice): Promise<string> {
  const spaceId = uid("space");
  const res = await call("PUT", `/spaces/${spaceId}`, { token: owner.token });
  if (res.status !== 201) throw new Error(`claim failed: ${res.status}`);
  return spaceId;
}

/** Invites a user with `role` and accepts on their behalf. */
export async function addMember(owner: TestDevice, spaceId: string, role: "Editor" | "Viewer"): Promise<TestDevice> {
  const member = await newUser(role);
  const created = await call("POST", `/spaces/${spaceId}/invites`, { token: owner.token, body: { role } });
  const { token } = await created.json<{ token: string }>();
  const accepted = await call("POST", `/invites/${token}/accept`, { token: member.token });
  if (accepted.status !== 200) throw new Error(`accept failed: ${accepted.status}`);
  return member;
}

/** A sync socket with a message queue, so tests can await the next server message in order. */
export class SyncSocket {
  private readonly queue: ServerMessage[] = [];
  private waiter: ((message: ServerMessage) => void) | null = null;

  constructor(readonly ws: WebSocket) {
    ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data as string) as ServerMessage;
      if (this.waiter) {
        const resolve = this.waiter;
        this.waiter = null;
        resolve(message);
      } else {
        this.queue.push(message);
      }
    });
    ws.accept();
  }

  next(timeoutMs = 2000): Promise<ServerMessage> {
    const queued = this.queue.shift();
    if (queued) return Promise.resolve(queued);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiter = null;
        reject(new Error("timed out waiting for a server message"));
      }, timeoutMs);
      this.waiter = (message) => {
        clearTimeout(timer);
        resolve(message);
      };
    });
  }

  /** Reads the connect-time backlog up to the `synced` marker. */
  async backlog(): Promise<{ changes: StoredChange[]; seq: number }> {
    const changes: StoredChange[] = [];
    for (;;) {
      const message = await this.next();
      if (message.type === "changes") changes.push(...message.changes);
      else if (message.type === "synced") return { changes, seq: message.seq };
      else throw new Error(`unexpected ${message.type} during backlog`);
    }
  }

  push(changes: ClientChange[]): void {
    this.ws.send(JSON.stringify({ type: "push", changes }));
  }

  close(): void {
    this.ws.close(1000, "done");
  }
}

export async function connect(device: TestDevice, spaceId: string, since = 0): Promise<SyncSocket> {
  const res = await call("GET", `/spaces/${spaceId}/sync?since=${since}`, {
    token: device.token,
    headers: { Upgrade: "websocket" },
  });
  if (res.status !== 101 || !res.webSocket) throw new Error(`connect failed: ${res.status}`);
  return new SyncSocket(res.webSocket);
}

export function change(id: string, payload: unknown = { title: id }): ClientChange {
  return { change_id: id, hlc: `0000000001:${id}`, payload };
}
