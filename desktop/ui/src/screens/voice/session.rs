//! What the voice parts draw, from the call (gelabber_call::CallState) and
//! the gateway's roster: whether this client sits in a room, who is there
//! with which flags, who speaks, and the pictures of the call (web
//! VoiceRoom.tsx state model, voice/roster.ts, voice/speaking.ts).

use gelabber_call::{CallState, CallStatus, VideoKey};
use gelabber_client::store::collation_key;
use gelabber_client::{Permission, State, VoiceOccupant};
use uuid::Uuid;

use super::stage::{Kind, PicId};
use crate::app::AppCtx;

/// The call's room while one runs (`voice.status === "joined"`).
pub fn joined(call: &CallState) -> Option<(Uuid, Uuid)> {
    if call.status == CallStatus::Idle {
        return None;
    }
    Some((call.server?, call.channel?))
}

/// The own seat on `server`: the call's, else the gateway's.
pub fn own_seat(app: &AppCtx, server: Uuid) -> Option<(Uuid, bool, bool)> {
    let call = app
        .call
        .with_state(|c| joined(c).map(|(s, ch)| (s, ch, c.muted, c.deafened)));
    match call {
        Some((s, ch, muted, deafened)) if s == server => Some((ch, muted, deafened)),
        Some(_) => None,
        None => app.client.with_state(|s| {
            (s.voice_self.server == Some(server))
                .then_some(s.voice_self.channel)
                .flatten()
                .map(|ch| (ch, s.voice_self.muted, s.voice_self.deafened))
        }),
    }
}

/// Above the threshold and not silenced (`VoiceRoom.tsx` `speaks`).
pub fn speaking(call: &CallState, user: Uuid, muted: bool, deafened: bool) -> bool {
    !muted
        && !deafened
        && call
            .participants
            .iter()
            .any(|p| p.user == user && p.speaking)
}

/// Who sits in `channel`, sorted by name: the roster, the own seat from
/// the call (the local join shows before its echo), and the own flags as
/// the call has them. `with_me` keeps the own seat while the call is
/// elsewhere (the sidebar lists the roster as it is).
pub fn occupants(
    s: &State,
    call: &CallState,
    server: Uuid,
    channel: Uuid,
    with_me: bool,
) -> Vec<(Uuid, VoiceOccupant)> {
    let me = s.me().map(|m| m.id);
    let here = joined(call) == Some((server, channel));
    let mut out = s.voice_occupants(server, channel);
    if let Some(me) = me {
        if here {
            let own = VoiceOccupant {
                channel_id: channel,
                muted: call.muted,
                deafened: call.deafened,
            };
            match out.iter_mut().find(|(u, _)| *u == me) {
                Some(entry) => entry.1 = own,
                None => {
                    out.push((me, own));
                    let name = |u: &Uuid| {
                        s.detail(server)
                            .and_then(|d| d.members.iter().find(|m| m.user_id == *u))
                            .map(|m| collation_key(&m.name))
                            .unwrap_or_default()
                    };
                    out.sort_by_cached_key(|(u, _)| (name(u), *u));
                }
            }
        } else if !with_me || joined(call).is_some_and(|(sv, _)| sv == server) {
            // The roster still lists a seat the call has left or moved.
            out.retain(|(u, _)| *u != me);
        }
    }
    out
}

/// One person in a room.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub id: Uuid,
    /// The member's name, the own account's, or "Mitglied".
    pub name: String,
    pub avatar: Option<String>,
    pub me: bool,
    /// A member record exists: the profile and "…" are offered.
    pub member: bool,
    pub muted: bool,
    pub deafened: bool,
    pub speaking: bool,
    pub live: bool,
    pub camera: bool,
}

impl Person {
    /// `Taub` / `Stumm` / `Spricht` / `Im Raum` + ` · Live` / ` · Kamera an`.
    pub fn status(&self) -> String {
        let state = if self.deafened {
            "Taub"
        } else if self.muted {
            "Stumm"
        } else if self.speaking {
            "Spricht"
        } else {
            "Im Raum"
        };
        let media = if self.live {
            " · Live"
        } else if self.camera {
            " · Kamera an"
        } else {
            ""
        };
        format!("{state}{media}")
    }
}

/// A camera or a Live on the stage.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub id: PicId,
    pub key: Option<VideoKey>,
    /// `{name} — Live`, or the name for a camera (`(du)` for the own).
    pub label: String,
    pub mirror: bool,
    pub speaking: bool,
    /// Someone else's Live that this client watches.
    pub watching: bool,
    pub own: bool,
}

/// The room as the page and the overview draw it.
#[derive(Clone, Debug, Default)]
pub struct Room {
    pub server: Uuid,
    pub channel: Uuid,
    pub channel_name: String,
    pub here: bool,
    /// In another call: the button says "Hierher wechseln".
    pub elsewhere: bool,
    pub elsewhere_name: Option<String>,
    pub allowed: bool,
    pub can_live: bool,
    pub me: Option<Uuid>,
    pub occupants: Vec<Person>,
    pub shares: Vec<Picture>,
    pub cameras: Vec<Picture>,
    /// Names of the others (`Jemand` without a member), for the summary.
    pub others: Vec<String>,
    pub others_live: Vec<String>,
    pub own_live: bool,
    /// A watched Live of someone else plays (the stream sound control).
    pub watching_any: bool,
}

impl Room {
    pub fn staged(&self) -> bool {
        !self.shares.is_empty() || !self.cameras.is_empty()
    }

    pub fn person(&self, id: Uuid) -> Option<&Person> {
        self.occupants.iter().find(|p| p.id == id)
    }

    /// `nameOf`: `{name} (du)` for the own account.
    pub fn name_of(&self, id: Uuid) -> String {
        match self.person(id) {
            Some(p) if p.me => format!("{} (du)", p.name),
            Some(p) => p.name.clone(),
            None => "Mitglied".into(),
        }
    }
}

/// The room of `channel` on `server`, or `None` while the server loads.
pub fn room(app: &AppCtx, server: Uuid, channel: Uuid) -> Option<Room> {
    let call = app.call.state();
    app.client.with_state(|s| build(s, &call, server, channel))
}

/// [`room`] from the two states (tests build them by hand).
pub fn build(s: &State, call: &CallState, server: Uuid, channel: Uuid) -> Option<Room> {
    let detail = s.detail(server)?;
    let channel_name = detail
        .channels
        .iter()
        .find(|c| c.id == channel)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let me_user = s.me();
    let me = me_user.map(|u| u.id);
    let current = joined(call);
    let here = current == Some((server, channel));
    let elsewhere = current.is_some() && !here;
    let elsewhere_name = current.and_then(|(_, ch)| s.channel(ch).map(|c| c.name.clone()));
    let view = s.server(server);
    let allowed = view.is_none_or(|v| v.can(Permission::JoinVoice));
    let can_live = view.is_none_or(|v| v.can(Permission::GoLive));
    let live_order: Vec<Uuid> = s
        .live
        .get(&server)
        .and_then(|l| l.get(&channel))
        .cloned()
        .unwrap_or_default();
    // The web keeps the roster's order: the server's snapshots list seats
    // by user id, so the room does too.
    let mut seats = occupants(s, call, server, channel, false);
    seats.sort_by_key(|(u, _)| *u);
    let participant = |u: Uuid| call.participants.iter().find(|p| p.user == u);
    let mut room = Room {
        server,
        channel,
        channel_name,
        here,
        elsewhere,
        elsewhere_name,
        allowed,
        can_live,
        me,
        own_live: here && call.live,
        ..Room::default()
    };
    for (id, flags) in seats {
        let member = detail.members.iter().find(|m| m.user_id == id);
        let own = Some(id) == me;
        let name = member.map(|m| m.name.clone()).unwrap_or_else(|| {
            if own {
                me_user.map_or("Du".into(), |u| u.name.clone())
            } else {
                "Mitglied".into()
            }
        });
        let avatar = member.and_then(|m| m.avatar_url.clone()).or_else(|| {
            own.then(|| me_user.and_then(|u| u.avatar_url.clone()))
                .flatten()
        });
        let p = participant(id);
        let camera = here
            && if own {
                call.camera
            } else {
                p.is_some_and(|p| p.camera.is_some())
            };
        let sharing = here
            && if own {
                call.live
            } else {
                p.is_some_and(|p| p.live.is_some())
            };
        let live = live_order.contains(&id) || sharing;
        let speaks = speaking(call, id, flags.muted, flags.deafened);
        room.occupants.push(Person {
            id,
            name: name.clone(),
            avatar,
            me: own,
            member: member.is_some(),
            muted: flags.muted,
            deafened: flags.deafened,
            speaking: speaks,
            live,
            camera,
        });
        if !own {
            let summary_name = member.map_or("Jemand".to_owned(), |m| m.name.clone());
            room.others.push(summary_name.clone());
            if live {
                room.others_live.push(summary_name);
            }
        }
        let shown = if own { format!("{name} (du)") } else { name };
        if sharing {
            room.shares.push(Picture {
                id: PicId {
                    user: id,
                    kind: Kind::Live,
                },
                key: p.and_then(|p| p.live),
                label: format!("{shown} — Live"),
                mirror: false,
                speaking: false,
                watching: !own && p.is_some_and(|p| p.watching),
                own,
            });
        }
        if camera {
            room.cameras.push(Picture {
                id: PicId {
                    user: id,
                    kind: Kind::Camera,
                },
                key: p.and_then(|p| p.camera),
                label: shown,
                mirror: own,
                speaking: speaks,
                watching: false,
                own,
            });
        }
    }
    // Whoever went live first keeps the spotlight when another joins in.
    let rank = |u: &Uuid| {
        live_order
            .iter()
            .position(|x| x == u)
            .unwrap_or(live_order.len())
    };
    room.shares.sort_by_key(|t| rank(&t.id.user));
    room.watching_any = room.shares.iter().any(|t| t.watching);
    Some(room)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gelabber_call::Participant;

    #[test]
    fn joined_needs_a_running_call() {
        let mut call = CallState::default();
        assert_eq!(joined(&call), None);
        call.server = Some(Uuid::from_u128(1));
        call.channel = Some(Uuid::from_u128(2));
        assert_eq!(joined(&call), None);
        call.status = CallStatus::Connecting;
        assert!(joined(&call).is_some());
    }

    #[test]
    fn silenced_people_do_not_speak() {
        let user = Uuid::from_u128(5);
        let call = CallState {
            participants: vec![Participant {
                user,
                speaking: true,
                ..Participant::default()
            }],
            ..CallState::default()
        };
        assert!(speaking(&call, user, false, false));
        assert!(!speaking(&call, user, true, false));
        assert!(!speaking(&call, user, false, true));
        assert!(!speaking(&call, Uuid::from_u128(6), false, false));
    }

    #[test]
    fn status_lines() {
        let mut p = Person {
            id: Uuid::nil(),
            name: "Mia".into(),
            avatar: None,
            me: false,
            member: true,
            muted: false,
            deafened: false,
            speaking: true,
            live: false,
            camera: true,
        };
        assert_eq!(p.status(), "Spricht · Kamera an");
        p.muted = true;
        p.live = true;
        assert_eq!(p.status(), "Stumm · Live");
    }
}
