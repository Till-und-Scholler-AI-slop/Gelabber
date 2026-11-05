//! The room model against hand-built states: the gateway's roster plus a
//! running call (gelabber_call's `testing` setter), as voice-joined has it.

use gelabber_call::{CallState, CallStatus, Participant, VideoKey};
use gelabber_client::{ServerDetail, Session, State, User, VoiceOccupant};
use serde_json::json;
use uuid::Uuid;

use super::session;
use super::stage::Kind;

const SERVER: Uuid = Uuid::from_u128(0x10);
const SOFA: Uuid = Uuid::from_u128(0x20);
const KINO: Uuid = Uuid::from_u128(0x21);
// Lena's id sorts first, as in the fixture.
const LENA: Uuid = Uuid::from_u128(0x37);
const JONAS: Uuid = Uuid::from_u128(0x7c);
const MIRA: Uuid = Uuid::from_u128(0xb8);

fn member(id: Uuid, name: &str) -> serde_json::Value {
    json!({
        "user_id": id, "name": name, "avatar_url": null,
        "joined_at": "2026-01-01T00:00:00Z", "status_text": null,
        "accent_color": null, "avatar_decoration": null, "role": "member"
    })
}

fn channel(id: Uuid, name: &str) -> serde_json::Value {
    json!({
        "id": id, "server_id": SERVER, "category_id": null, "name": name,
        "kind": "voice", "created_at": "2026-01-01T00:00:00Z"
    })
}

/// Wohnzimmer with Jonas and Mira (muted) in Sofa; Lena signed in.
fn state() -> State {
    let server = json!({
        "id": SERVER, "name": "Wohnzimmer", "owner_id": LENA, "icon_url": null,
        "created_at": "2026-01-01T00:00:00Z", "role": "member",
        "permissions": ["join_voice", "go_live"], "member_permissions": []
    });
    let mut detail = server.clone();
    detail["categories"] = json!([]);
    detail["channels"] = json!([channel(SOFA, "Sofa"), channel(KINO, "Kino")]);
    detail["members"] = json!([
        member(LENA, "Lena"),
        member(JONAS, "Jonas"),
        member(MIRA, "Mira")
    ]);
    let detail: ServerDetail = serde_json::from_value(detail).expect("detail");
    let user: User = serde_json::from_value(json!({
        "id": LENA, "email": "lena@example.test", "name": "Lena", "avatar_url": null,
        "created_at": "2026-01-01T00:00:00Z", "bio": null, "pronouns": null,
        "status_text": null, "accent_color": null, "banner_color": null,
        "banner_url": null, "avatar_decoration": null
    }))
    .expect("user");
    let mut s = State::default();
    s.session = Session::Authenticated(Box::new(user));
    s.servers = vec![serde_json::from_value(server).expect("server")];
    s.server_details.insert(SERVER, detail);
    let seat = |muted| VoiceOccupant {
        channel_id: SOFA,
        muted,
        deafened: false,
    };
    s.voice
        .insert(SERVER, [(JONAS, seat(false)), (MIRA, seat(true))].into());
    s
}

/// The call of voice-joined: Lena and Jonas speak, Mira is muted.
fn joined() -> CallState {
    CallState {
        status: CallStatus::Connected,
        server: Some(SERVER),
        channel: Some(SOFA),
        quality: 4,
        participants: vec![
            Participant {
                user: LENA,
                me: true,
                speaking: true,
                ..Participant::default()
            },
            Participant {
                user: JONAS,
                speaking: true,
                ..Participant::default()
            },
            Participant {
                user: MIRA,
                muted: true,
                speaking: true,
                ..Participant::default()
            },
        ],
        ..CallState::default()
    }
}

#[test]
fn outside_the_room() {
    let room = session::build(&state(), &CallState::default(), SERVER, SOFA).expect("room");
    assert!(!room.here && !room.elsewhere && room.allowed);
    let names: Vec<&str> = room.occupants.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Jonas", "Mira"]);
    assert_eq!(room.occupants[1].status(), "Stumm");
    assert_eq!(
        super::summary::room_summary(&room.others, room.here, &room.others_live),
        "Jonas und Mira sind da."
    );
    assert!(!room.staged());
    let empty = session::build(&state(), &CallState::default(), SERVER, KINO).expect("room");
    assert!(empty.occupants.is_empty());
}

#[test]
fn inside_the_room() {
    let call = joined();
    let room = session::build(&state(), &call, SERVER, SOFA).expect("room");
    assert!(room.here);
    // The local join shows before its echo, in the roster's order.
    let names: Vec<&str> = room.occupants.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Lena", "Jonas", "Mira"]);
    assert!(room.occupants[0].me && room.occupants[0].speaking);
    assert_eq!(room.occupants[0].status(), "Spricht");
    // Muted people never speak, whatever the detector says.
    assert!(!room.occupants[2].speaking);
    assert_eq!(
        super::summary::room_summary(&room.others, room.here, &room.others_live),
        "Du bist hier mit Jonas und Mira."
    );
    // The sidebar lists Lena among them too; the call's other room sees
    // the switch.
    let s = state();
    let sidebar = session::occupants(&s, &call, SERVER, SOFA, true);
    assert_eq!(sidebar.len(), 3);
    let other = session::build(&s, &call, SERVER, KINO).expect("room");
    assert!(other.elsewhere);
    assert_eq!(other.elsewhere_name.as_deref(), Some("Sofa"));
}

#[test]
fn pictures_of_the_call() {
    let mut call = joined();
    call.camera = true;
    call.participants[1].camera = Some(VideoKey(7));
    call.participants[2].live = Some(VideoKey(8));
    let mut s = state();
    s.live.insert(SERVER, [(SOFA, vec![MIRA])].into());
    let room = session::build(&s, &call, SERVER, SOFA).expect("room");
    assert!(room.staged());
    assert_eq!(room.shares.len(), 1);
    assert_eq!(room.shares[0].label, "Mira — Live");
    assert_eq!(room.shares[0].id.kind, Kind::Live);
    let cams: Vec<&str> = room.cameras.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(cams, ["Lena (du)", "Jonas"]);
    assert!(room.cameras[0].mirror && !room.cameras[1].mirror);
    assert_eq!(room.cameras[1].key, Some(VideoKey(7)));
    assert_eq!(room.others_live, ["Mira"]);
    assert_eq!(
        super::summary::stage_facts(
            room.occupants.len(),
            room.cameras.len(),
            &room.others_live,
            room.own_live
        ),
        "3 im Kanal · Mira ist live"
    );
    // Outside the call no pictures run: a Live is watched in the room.
    let outside = session::build(&s, &CallState::default(), SERVER, SOFA).expect("room");
    assert!(!outside.staged());
    assert_eq!(outside.others_live, ["Mira"]);
}
