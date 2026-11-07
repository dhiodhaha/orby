import { describe, expect, it } from "vitest";
import { INVITE_TTL_MS } from "../src/invites";
import { NOW, addMember, call, change, connect, newSpace, newUser } from "./helpers";

async function createInvite(token: string, spaceId: string, role: unknown, now = NOW) {
  return call("POST", `/spaces/${spaceId}/invites`, { token, body: { role }, now });
}

describe("invites", () => {
  it("joins the space with the invite's role, and the new editor can push", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const ibu = await newUser("Ibu");

    const created = await createInvite(owner.token, spaceId, "Editor");
    expect(created.status).toBe(201);
    const invite = await created.json<{ token: string; role: string; expiresAt: number; spaceId: string }>();
    expect(invite).toMatchObject({ spaceId, role: "Editor", expiresAt: NOW + INVITE_TTL_MS });

    const accepted = await call("POST", `/invites/${invite.token}/accept`, { token: ibu.token, now: NOW + 60_000 });
    expect(accepted.status).toBe(200);
    expect(await accepted.json()).toEqual({ spaceId, role: "Editor" });

    const socket = await connect(ibu, spaceId);
    await socket.backlog();
    socket.push([change("c1")]);
    expect(await socket.next()).toEqual({ type: "ack", seq: 1, duplicates: [] });
    socket.close();
  });

  it("fails when the same token is used a second time", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const first = await newUser("Ibu");
    const second = await newUser("Kakak");
    const { token } = await (await createInvite(owner.token, spaceId, "Viewer")).json<{ token: string }>();

    expect((await call("POST", `/invites/${token}/accept`, { token: first.token })).status).toBe(200);
    const reused = await call("POST", `/invites/${token}/accept`, { token: second.token });
    expect(reused.status).toBe(409);
    expect(await reused.json()).toEqual({ error: "invite_used" });

    const denied = await call("GET", `/spaces/${spaceId}/sync`, {
      token: second.token,
      headers: { Upgrade: "websocket" },
    });
    expect(denied.status).toBe(403);
  });

  it("is valid until 7 days after creation and expired from then on", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const early = await newUser("Ibu");
    const late = await newUser("Kakak");
    const a = await (await createInvite(owner.token, spaceId, "Editor")).json<{ token: string }>();
    const b = await (await createInvite(owner.token, spaceId, "Editor")).json<{ token: string }>();

    const justInTime = await call("POST", `/invites/${a.token}/accept`, {
      token: early.token,
      now: NOW + INVITE_TTL_MS - 1,
    });
    expect(justInTime.status).toBe(200);

    const tooLate = await call("POST", `/invites/${b.token}/accept`, { token: late.token, now: NOW + INVITE_TTL_MS });
    expect(tooLate.status).toBe(410);
    expect(await tooLate.json()).toEqual({ error: "invite_expired" });
  });

  it("does not use up the invite when an existing member opens it", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const newcomer = await newUser("Ibu");
    const { token } = await (await createInvite(owner.token, spaceId, "Editor")).json<{ token: string }>();

    const self = await call("POST", `/invites/${token}/accept`, { token: owner.token });
    expect(self.status).toBe(409);
    expect(await self.json()).toEqual({ error: "already_member", spaceId, role: "Owner" });
    expect((await call("POST", `/invites/${token}/accept`, { token: newcomer.token })).status).toBe(200);
  });

  it("lets only the owner invite, and never as Owner", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const editor = await addMember(owner, spaceId, "Editor");
    const stranger = await newUser("Tetangga");

    expect((await createInvite(editor.token, spaceId, "Viewer")).status).toBe(403);
    expect((await createInvite(stranger.token, spaceId, "Viewer")).status).toBe(403);
    expect((await createInvite(owner.token, spaceId, "Owner")).status).toBe(400);
    expect((await createInvite(owner.token, spaceId, "admin")).status).toBe(400);
  });

  it("rejects an unknown token", async () => {
    const user = await newUser("Ibu");
    const res = await call("POST", "/invites/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA/accept", { token: user.token });
    expect(res.status).toBe(404);
  });
});

describe("space ownership", () => {
  it("gives a new space to the first claimer only", async () => {
    const owner = await newUser("Ayah");
    const other = await newUser("Tetangga");
    const spaceId = await newSpace(owner);

    expect((await call("PUT", `/spaces/${spaceId}`, { token: owner.token })).status).toBe(200);
    const taken = await call("PUT", `/spaces/${spaceId}`, { token: other.token });
    expect(taken.status).toBe(409);
    expect(await taken.json()).toEqual({ error: "space_taken" });
  });
});
