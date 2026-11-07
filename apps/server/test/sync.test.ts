import { evictDurableObject } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { addMember, call, change, connect, newDevice, newSpace, newUser } from "./helpers";

describe("sync relay", () => {
  it("acks a push with the new seq and broadcasts it to the other socket", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const phone = await newDevice(owner.userId);
    const editor = await addMember(owner, spaceId, "Editor");

    const a = await connect(owner, spaceId);
    const b = await connect(editor, spaceId);
    expect(await a.backlog()).toEqual({ changes: [], seq: 0 });
    expect(await b.backlog()).toEqual({ changes: [], seq: 0 });

    a.push([change("c1", { title: "Gas elpiji 3 kg" }), change("c2")]);
    expect(await a.next()).toEqual({ type: "ack", seq: 2, duplicates: [] });
    expect(await b.next()).toEqual({
      type: "changes",
      changes: [
        { seq: 1, change_id: "c1", device: owner.deviceId, hlc: "0000000001:c1", payload: { title: "Gas elpiji 3 kg" } },
        { seq: 2, change_id: "c2", device: owner.deviceId, hlc: "0000000001:c2", payload: { title: "c2" } },
      ],
    });

    // The pusher never gets its own changes back: its next message is the editor's change.
    const c = await connect(phone, spaceId, 2);
    await c.backlog();
    b.push([change("c3")]);
    expect(await b.next()).toMatchObject({ type: "ack", seq: 3 });
    expect(await a.next()).toMatchObject({ type: "changes", changes: [{ seq: 3, change_id: "c3", device: editor.deviceId }] });
    expect(await c.next()).toMatchObject({ type: "changes", changes: [{ seq: 3, change_id: "c3" }] });

    a.close();
    b.close();
    c.close();
  });

  it("sends only the backlog after ?since=N, then the head seq", async () => {
    const owner = await newUser("Ibu");
    const spaceId = await newSpace(owner);
    const writer = await connect(owner, spaceId);
    await writer.backlog();
    writer.push([change("c1"), change("c2"), change("c3")]);
    await writer.next();

    const late = await connect(owner, spaceId, 1);
    const { changes, seq } = await late.backlog();
    expect(changes.map((c) => [c.seq, c.change_id])).toEqual([
      [2, "c2"],
      [3, "c3"],
    ]);
    expect(seq).toBe(3);

    const current = await connect(owner, spaceId, 3);
    expect(await current.backlog()).toEqual({ changes: [], seq: 3 });

    writer.close();
    late.close();
    current.close();
  });

  it("ignores a duplicate change_id and keeps the first copy", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const editor = await addMember(owner, spaceId, "Editor");
    const a = await connect(owner, spaceId);
    const b = await connect(editor, spaceId);
    await a.backlog();
    await b.backlog();

    a.push([change("c1", { title: "first" })]);
    expect(await a.next()).toEqual({ type: "ack", seq: 1, duplicates: [] });
    await b.next();

    // A retried outbox entry arriving again, next to a new one.
    a.push([change("c1", { title: "retry" }), change("c2")]);
    expect(await a.next()).toEqual({ type: "ack", seq: 2, duplicates: ["c1"] });
    expect(await b.next()).toMatchObject({ type: "changes", changes: [{ seq: 2, change_id: "c2" }] });

    a.push([change("c2")]);
    expect(await a.next()).toEqual({ type: "ack", seq: 2, duplicates: ["c2"] });

    const fresh = await connect(owner, spaceId);
    const { changes } = await fresh.backlog();
    expect(changes.map((c) => c.change_id)).toEqual(["c1", "c2"]);
    expect(changes[0]?.payload).toEqual({ title: "first" });

    a.close();
    b.close();
    fresh.close();
  });

  it("lets a viewer read but rejects its push without storing anything", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const viewer = await addMember(owner, spaceId, "Viewer");

    const writer = await connect(owner, spaceId);
    await writer.backlog();
    writer.push([change("c1")]);
    await writer.next();

    const v = await connect(viewer, spaceId);
    expect((await v.backlog()).changes.map((c) => c.change_id)).toEqual(["c1"]);

    v.push([change("sneaky")]);
    expect(await v.next()).toMatchObject({ type: "error", code: "read_only" });

    const check = await connect(owner, spaceId);
    expect(await check.backlog()).toMatchObject({ seq: 1, changes: [{ change_id: "c1" }] });

    writer.close();
    v.close();
    check.close();
  });

  it("keeps relaying and enforcing roles after the object hibernates", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const viewer = await addMember(owner, spaceId, "Viewer");
    const a = await connect(owner, spaceId);
    const v = await connect(viewer, spaceId);
    await a.backlog();
    await v.backlog();

    await evictDurableObject(env.SPACE.get(env.SPACE.idFromName(spaceId)), { webSockets: "hibernate" });

    a.push([change("c1")]);
    expect(await a.next()).toEqual({ type: "ack", seq: 1, duplicates: [] });
    expect(await v.next()).toMatchObject({ type: "changes", changes: [{ seq: 1, device: owner.deviceId }] });
    v.push([change("c2")]);
    expect(await v.next()).toMatchObject({ type: "error", code: "read_only" });

    a.close();
    v.close();
  });

  it("answers a malformed message with an error and keeps the socket open", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const a = await connect(owner, spaceId);
    await a.backlog();

    a.ws.send("not json");
    expect(await a.next()).toMatchObject({ type: "error", code: "bad_message" });
    a.ws.send(JSON.stringify({ type: "push", changes: [{ change_id: "", hlc: "x", payload: 1 }] }));
    expect(await a.next()).toMatchObject({ type: "error", code: "bad_message" });

    a.push([change("c1")]);
    expect(await a.next()).toEqual({ type: "ack", seq: 1, duplicates: [] });
    a.close();
  });

  it("refuses to open a socket without a valid token or membership", async () => {
    const owner = await newUser("Ayah");
    const spaceId = await newSpace(owner);
    const stranger = await newUser("Tetangga");
    const upgrade = { Upgrade: "websocket" };
    const path = `/spaces/${spaceId}/sync?since=0`;

    expect((await call("GET", path, { headers: upgrade })).status).toBe(401);
    expect((await call("GET", path, { headers: upgrade, token: `${owner.token}x` })).status).toBe(401);
    expect((await call("GET", path, { headers: upgrade, token: stranger.token })).status).toBe(403);
    expect((await call("GET", path, { token: owner.token })).status).toBe(426);
    expect(
      (await call("GET", `/spaces/${spaceId}/sync?since=-1`, { headers: upgrade, token: owner.token })).status,
    ).toBe(400);
  });
});
