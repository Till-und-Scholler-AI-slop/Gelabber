import type { VoiceFlags } from "../voice/roster.ts";
import type { Channel, Member, ServerDetail } from "./types.ts";

type Connection = {
  status: string;
  serverId: string | null;
  channelId: string | null;
};

/** Prefer the current call, then the last visited room, then an occupied room. */
export function overviewRoom(
  server: Pick<ServerDetail, "id" | "channels" | "members">,
  roster: Record<string, VoiceFlags>,
  connection: Connection,
  remembered?: string,
): Channel | undefined {
  const rooms = server.channels.filter(
    (c) => c.kind === "voice" && !c.id.startsWith("tmp:"),
  );
  const active =
    connection.status === "joined" && connection.serverId === server.id
      ? rooms.find((c) => c.id === connection.channelId)
      : undefined;
  const knownMembers = new Set(server.members.map((member) => member.user_id));
  const occupied = new Set(
    Object.entries(roster)
      .filter(([id]) => knownMembers.has(id))
      .map(([, flags]) => flags.channelId),
  );
  return (
    active ??
    rooms.find((c) => c.id === remembered) ??
    rooms.find((c) => occupied.has(c.id)) ??
    rooms[0]
  );
}

/** Presence is occupancy, never a claim that someone is currently speaking. */
export function roomWelcome(
  members: Member[],
  roster: Record<string, VoiceFlags>,
  room: Channel,
  selfId: string,
): string {
  const names = members
    .filter(
      (member) =>
        member.user_id !== selfId &&
        roster[member.user_id]?.channelId === room.id,
    )
    .map((member) => member.name);
  if (names.length === 0) return `Mach es dir in „${room.name}“ gemütlich.`;
  if (names.length === 1) return `${names[0]} ist schon in „${room.name}“.`;
  if (names.length === 2)
    return `${names[0]} und ${names[1]} sind schon in „${room.name}“.`;
  return `${names[0]}, ${names[1]} und ${names.length - 2} weitere sind in „${room.name}“.`;
}
