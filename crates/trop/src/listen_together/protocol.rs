//! `Trop/ListenTogether/ListenTogetherProtocol.swift`: the messages of
//! metroserver, the Listen Together server Metrolist uses: protobuf
//! envelopes over a WebSocket. The schema is
//! github.com/MetrolistGroup/metroproto (listentogether.proto); the field
//! numbers below match it.

/// Message type names, as metroserver spells them.
pub mod kind {
    pub const CLIENT_CAPABILITIES: &str = "client_capabilities";
    pub const SERVER_CAPABILITIES: &str = "server_capabilities";
    pub const CREATE_ROOM: &str = "create_room";
    pub const JOIN_ROOM: &str = "join_room";
    pub const LEAVE_ROOM: &str = "leave_room";
    pub const APPROVE_JOIN: &str = "approve_join";
    pub const REJECT_JOIN: &str = "reject_join";
    pub const PLAYBACK_ACTION: &str = "playback_action";
    pub const BUFFER_READY: &str = "buffer_ready";
    pub const KICK_USER: &str = "kick_user";
    pub const TRANSFER_HOST: &str = "transfer_host";
    pub const PING: &str = "ping";
    pub const RECONNECT: &str = "reconnect";
    pub const SUGGEST_TRACK: &str = "suggest_track";
    pub const APPROVE_SUGGESTION: &str = "approve_suggestion";
    pub const REJECT_SUGGESTION: &str = "reject_suggestion";

    pub const ROOM_CREATED: &str = "room_created";
    pub const JOIN_REQUEST: &str = "join_request";
    pub const JOIN_APPROVED: &str = "join_approved";
    pub const JOIN_REJECTED: &str = "join_rejected";
    pub const USER_JOINED: &str = "user_joined";
    pub const USER_LEFT: &str = "user_left";
    pub const SYNC_PLAYBACK: &str = "sync_playback";
    pub const ERROR: &str = "error";
    pub const PONG: &str = "pong";
    pub const HOST_CHANGED: &str = "host_changed";
    pub const KICKED: &str = "kicked";
    pub const SYNC_STATE: &str = "sync_state";
    pub const RECONNECTED: &str = "reconnected";
    pub const USER_RECONNECTED: &str = "user_reconnected";
    pub const USER_DISCONNECTED: &str = "user_disconnected";
    pub const SUGGESTION_RECEIVED: &str = "suggestion_received";
    pub const SUGGESTION_APPROVED: &str = "suggestion_approved";
    pub const SUGGESTION_REJECTED: &str = "suggestion_rejected";
    pub const SUGGESTION_SUBMITTED: &str = "suggestion_submitted";
    pub const SERVER_RESTARTING: &str = "server_restarting";
}

// MARK: - Wire format

/// Writes the protobuf fields these messages use. Default values are left
/// out, as protobuf itself does.
#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    fn varint(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.0.push((value & 0x7F) as u8 | 0x80);
            value >>= 7;
        }
        self.0.push(value as u8);
    }

    fn tag(&mut self, field: u32, wire: u64) {
        self.varint(u64::from(field) << 3 | wire);
    }

    pub fn uint(&mut self, field: u32, value: u64) {
        if value != 0 {
            self.tag(field, 0);
            self.varint(value);
        }
    }

    pub fn int(&mut self, field: u32, value: i64) {
        self.uint(field, value as u64);
    }

    pub fn bool(&mut self, field: u32, value: bool) {
        self.uint(field, u64::from(value));
    }

    pub fn bytes(&mut self, field: u32, value: &[u8]) {
        if !value.is_empty() {
            self.tag(field, 2);
            self.varint(value.len() as u64);
            self.0.extend_from_slice(value);
        }
    }

    pub fn string(&mut self, field: u32, value: &str) {
        self.bytes(field, value.as_bytes());
    }

    /// An embedded message, written even when empty.
    pub fn message(&mut self, field: u32, value: &[u8]) {
        self.tag(field, 2);
        self.varint(value.len() as u64);
        self.0.extend_from_slice(value);
    }

    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("malformed protobuf")
    }
}

impl std::error::Error for Malformed {}

#[derive(Debug, Clone, Copy)]
pub enum Value<'a> {
    Varint(u64),
    Fixed32(u32),
    Fixed64(u64),
    Bytes(&'a [u8]),
}

impl<'a> Value<'a> {
    pub fn uint(self) -> u64 {
        match self {
            Value::Varint(v) => v,
            _ => 0,
        }
    }

    pub fn int(self) -> i64 {
        self.uint() as i64
    }

    pub fn bool(self) -> bool {
        self.uint() != 0
    }

    pub fn data(self) -> &'a [u8] {
        match self {
            Value::Bytes(b) => b,
            _ => &[],
        }
    }

    pub fn string(self) -> String {
        String::from_utf8_lossy(self.data()).into_owned()
    }
}

/// Calls `body` with each field number and value, in order. Unknown fields
/// are the caller's to skip, so newer servers can add some.
pub fn fields<'a>(
    data: &'a [u8],
    mut body: impl FnMut(u32, Value<'a>) -> Result<(), Malformed>,
) -> Result<(), Malformed> {
    let mut at = 0;
    let varint = |at: &mut usize| -> Result<u64, Malformed> {
        let mut result = 0u64;
        let mut shift = 0;
        loop {
            let byte = *data.get(*at).ok_or(Malformed)?;
            *at += 1;
            if shift >= 64 {
                return Err(Malformed);
            }
            result |= u64::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    };
    let take = |at: &mut usize, count: usize| -> Result<&'a [u8], Malformed> {
        let end = at.checked_add(count).ok_or(Malformed)?;
        let slice = data.get(*at..end).ok_or(Malformed)?;
        *at = end;
        Ok(slice)
    };
    while at < data.len() {
        let key = varint(&mut at)?;
        let field = (key >> 3) as u32;
        let value = match key & 7 {
            0 => Value::Varint(varint(&mut at)?),
            1 => Value::Fixed64(u64::from_le_bytes(
                take(&mut at, 8)?.try_into().map_err(|_| Malformed)?,
            )),
            2 => {
                let length = varint(&mut at)? as usize;
                Value::Bytes(take(&mut at, length)?)
            }
            5 => Value::Fixed32(u32::from_le_bytes(
                take(&mut at, 4)?.try_into().map_err(|_| Malformed)?,
            )),
            _ => return Err(Malformed),
        };
        body(field, value)?;
    }
    Ok(())
}

// MARK: - Envelope

/// Every WebSocket frame: the message type and its encoded payload.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Envelope {
    pub kind: String,
    pub payload: Vec<u8>,
    /// The server gzips payloads only for clients that ask; this one doesn't.
    pub compressed: bool,
}

impl Envelope {
    pub fn encode(kind: &str, payload: &[u8]) -> Vec<u8> {
        let mut w = Writer::default();
        w.string(1, kind);
        w.bytes(2, payload);
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self, Malformed> {
        let mut envelope = Self::default();
        fields(data, |field, v| {
            match field {
                1 => envelope.kind = v.string(),
                2 => envelope.payload = v.data().to_vec(),
                3 => envelope.compressed = v.bool(),
                _ => {}
            }
            Ok(())
        })?;
        Ok(envelope)
    }
}

// MARK: - Shared

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Milliseconds.
    pub duration: i64,
    pub thumbnail: String,
    pub suggested_by: String,
}

impl Track {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.string(1, &self.id);
        w.string(2, &self.title);
        w.string(3, &self.artist);
        w.string(4, &self.album);
        w.int(5, self.duration);
        w.string(6, &self.thumbnail);
        w.string(7, &self.suggested_by);
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self, Malformed> {
        let mut t = Self::default();
        fields(data, |field, v| {
            match field {
                1 => t.id = v.string(),
                2 => t.title = v.string(),
                3 => t.artist = v.string(),
                4 => t.album = v.string(),
                5 => t.duration = v.int(),
                6 => t.thumbnail = v.string(),
                7 => t.suggested_by = v.string(),
                _ => {}
            }
            Ok(())
        })?;
        Ok(t)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct User {
    pub user_id: String,
    pub username: String,
    pub is_host: bool,
    pub is_connected: bool,
}

impl User {
    fn decode(data: &[u8]) -> Result<Self, Malformed> {
        let mut u = Self::default();
        fields(data, |field, v| {
            match field {
                1 => u.user_id = v.string(),
                2 => u.username = v.string(),
                3 => u.is_host = v.bool(),
                4 => u.is_connected = v.bool(),
                _ => {}
            }
            Ok(())
        })?;
        Ok(u)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoomState {
    pub host_id: String,
    pub users: Vec<User>,
    pub current_track: Option<Track>,
    pub queue: Vec<Track>,
}

impl RoomState {
    fn decode(data: &[u8]) -> Result<Self, Malformed> {
        let mut s = Self::default();
        fields(data, |field, v| {
            match field {
                2 => s.host_id = v.string(),
                3 => s.users.push(User::decode(v.data())?),
                4 => s.current_track = Some(Track::decode(v.data())?),
                9 => s.queue.push(Track::decode(v.data())?),
                _ => {}
            }
            Ok(())
        })?;
        Ok(s)
    }
}

// MARK: - Client to server

pub fn client_capabilities(version: &str, features: &[&str]) -> Vec<u8> {
    let mut w = Writer::default();
    w.bool(1, true); // supports_protobuf
    w.bool(2, false); // supports_compression
    w.string(3, version);
    for feature in features {
        w.string(4, feature);
    }
    w.finish()
}

/// create_room (`username`) and reconnect (`session_token`): one string in field 1.
pub fn one_string(value: &str) -> Vec<u8> {
    let mut w = Writer::default();
    w.string(1, value);
    w.finish()
}

/// join_room, and the user and suggestion actions (an ID with a reason):
/// two strings.
pub fn two_strings(first: &str, second: &str) -> Vec<u8> {
    let mut w = Writer::default();
    w.string(1, first);
    w.string(2, second);
    w.finish()
}

pub fn suggest_track(track: &Track) -> Vec<u8> {
    let mut w = Writer::default();
    w.message(1, &track.encode());
    w.finish()
}

pub fn buffer_ready(track_id: &str, revision: u64) -> Vec<u8> {
    let mut w = Writer::default();
    w.string(1, track_id);
    w.uint(2, revision);
    w.finish()
}

/// A ping; a guest playing a loaded song adds a drift report: where it is
/// in `track_id`, sampled at `captured_at` on the server's clock.
pub fn ping(client_time: i64, sequence: u64, drift: Option<(&str, i64, i64)>) -> Vec<u8> {
    let mut w = Writer::default();
    w.int(1, client_time);
    w.uint(2, sequence);
    if let Some((track_id, position, captured_at)) = drift {
        w.bool(3, true);
        w.string(4, track_id);
        w.int(5, position);
        w.int(6, captured_at);
    }
    w.finish()
}

// MARK: - Both ways

/// playback_action (host to server) and sync_playback (server to everyone).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playback {
    /// play, pause, seek, change_track, queue_add, queue_remove, queue_clear, sync_queue.
    pub action: String,
    pub track_id: String,
    /// Milliseconds.
    pub position: i64,
    pub track: Option<Track>,
    pub queue: Vec<Track>,
    /// Server clock (ms) when the server sent it.
    pub server_time: i64,
    pub revision: u64,
    /// Server clock (ms) when the host read `position`.
    pub captured_at: i64,
    pub replace_queue: bool,
    pub origin_user_id: String,
    pub host_sequence: u64,
}

impl Playback {
    pub fn new(action: &str, host_sequence: u64) -> Self {
        Self {
            action: action.to_owned(),
            host_sequence,
            ..Self::default()
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.string(1, &self.action);
        w.string(2, &self.track_id);
        w.int(3, self.position);
        if let Some(track) = &self.track {
            w.message(4, &track.encode());
        }
        for item in &self.queue {
            w.message(6, &item.encode());
        }
        w.uint(10, self.revision);
        w.int(11, self.captured_at);
        w.bool(12, self.replace_queue);
        w.uint(14, self.host_sequence);
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self, Malformed> {
        let mut p = Self::default();
        fields(data, |field, v| {
            match field {
                1 => p.action = v.string(),
                2 => p.track_id = v.string(),
                3 => p.position = v.int(),
                4 => p.track = Some(Track::decode(v.data())?),
                6 => p.queue.push(Track::decode(v.data())?),
                9 => p.server_time = v.int(),
                10 => p.revision = v.uint(),
                11 => p.captured_at = v.int(),
                12 => p.replace_queue = v.bool(),
                13 => p.origin_user_id = v.string(),
                14 => p.host_sequence = v.uint(),
                _ => {}
            }
            Ok(())
        })?;
        Ok(p)
    }
}

// MARK: - Server to client

/// room_created, join_approved and reconnected: where this client now is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Joined {
    pub room_code: String,
    pub user_id: String,
    pub session_token: String,
    pub state: Option<RoomState>,
    pub is_host: bool,
}

impl Joined {
    /// The three messages number the same fields differently; `kind` names which.
    pub fn decode(kind: &str, data: &[u8]) -> Result<Self, Malformed> {
        let created = kind == self::kind::ROOM_CREATED;
        let reconnected = kind == self::kind::RECONNECTED;
        let mut j = Self {
            is_host: created,
            ..Self::default()
        };
        fields(data, |field, v| {
            match field {
                1 => j.room_code = v.string(),
                2 => j.user_id = v.string(),
                3 if reconnected => j.state = Some(RoomState::decode(v.data())?),
                3 => j.session_token = v.string(),
                4 if reconnected => j.is_host = v.bool(),
                4 if !created => j.state = Some(RoomState::decode(v.data())?),
                _ => {}
            }
            Ok(())
        })?;
        Ok(j)
    }
}

/// join_request, user_joined, user_left, user_reconnected,
/// user_disconnected, host_changed: a user ID and name. Also error (code,
/// message), join_rejected and kicked (a reason in field 1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pair(pub String, pub String);

impl Pair {
    pub fn decode(data: &[u8]) -> Result<Self, Malformed> {
        let mut pair = Self::default();
        fields(data, |field, v| {
            match field {
                1 => pair.0 = v.string(),
                2 => pair.1 = v.string(),
                _ => {}
            }
            Ok(())
        })?;
        Ok(pair)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncState {
    pub current_track: Option<Track>,
    pub queue: Vec<Track>,
}

impl SyncState {
    pub fn decode(data: &[u8]) -> Result<Self, Malformed> {
        let mut s = Self::default();
        fields(data, |field, v| {
            match field {
                1 => s.current_track = Some(Track::decode(v.data())?),
                5 => s.queue.push(Track::decode(v.data())?),
                _ => {}
            }
            Ok(())
        })?;
        Ok(s)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pong {
    pub client_time: i64,
    pub server_receive_time: i64,
    pub server_send_time: i64,
    pub drift_valid: bool,
    /// This listener's position minus the room's (positive = ahead).
    pub drift_ms: i64,
}

impl Pong {
    pub fn decode(data: &[u8]) -> Result<Self, Malformed> {
        let mut p = Self::default();
        fields(data, |field, v| {
            match field {
                1 => p.client_time = v.int(),
                2 => p.server_receive_time = v.int(),
                3 => p.server_send_time = v.int(),
                5 => p.drift_valid = v.bool(),
                6 => p.drift_ms = v.int(),
                _ => {}
            }
            Ok(())
        })?;
        Ok(p)
    }
}

/// suggestion_received, suggestion_approved, suggestion_rejected and suggestion_submitted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Suggestion {
    pub id: String,
    pub from_username: String,
    pub track: Track,
}

impl Suggestion {
    pub fn decode(kind: &str, data: &[u8]) -> Result<Self, Malformed> {
        let received = kind == self::kind::SUGGESTION_RECEIVED;
        let mut s = Self::default();
        fields(data, |field, v| {
            match (field, received) {
                (1, _) => s.id = v.string(),
                (3, true) => s.from_username = v.string(),
                (4, true) => s.track = Track::decode(v.data())?,
                (2, false) if kind != self::kind::SUGGESTION_REJECTED => {
                    s.track = Track::decode(v.data())?;
                }
                _ => {}
            }
            Ok(())
        })?;
        Ok(s)
    }
}

/// server_restarting: when to come back, in ms.
pub fn restart_eta(data: &[u8]) -> Result<i64, Malformed> {
    let mut eta = 0;
    fields(data, |field, v| {
        if field == 1 {
            eta = v.int();
        }
        Ok(())
    })?;
    Ok(eta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playback_reads_back_what_it_wrote() {
        let track = Track {
            id: "abc".into(),
            title: "Hello".into(),
            artist: "Adele".into(),
            duration: 295_000,
            ..Track::default()
        };
        let mut playback = Playback::new("change_track", 7);
        playback.track = Some(track.clone());
        playback.queue = vec![track.clone(), track];
        playback.position = -1; // a negative int is ten varint bytes
        playback.replace_queue = true;
        let envelope = Envelope::encode(kind::PLAYBACK_ACTION, &playback.encode());
        let read = Envelope::decode(&envelope).unwrap();
        assert_eq!(read.kind, "playback_action");
        assert_eq!(Playback::decode(&read.payload).unwrap(), playback);
    }

    #[test]
    fn joined_messages_number_their_fields_differently() {
        let mut state = Writer::default();
        state.string(2, "host");
        let state = state.finish();

        let mut approved = Writer::default();
        approved.string(1, "ABCD");
        approved.string(2, "me");
        approved.string(3, "token");
        approved.message(4, &state);
        let approved = Joined::decode(kind::JOIN_APPROVED, &approved.finish()).unwrap();
        assert_eq!(approved.session_token, "token");
        assert_eq!(approved.state.unwrap().host_id, "host");
        assert!(!approved.is_host);

        let mut back = Writer::default();
        back.string(1, "ABCD");
        back.message(3, &state);
        back.bool(4, true);
        let back = Joined::decode(kind::RECONNECTED, &back.finish()).unwrap();
        assert!(back.session_token.is_empty() && back.is_host);
        assert_eq!(back.state.unwrap().host_id, "host");

        let created = Joined::decode(kind::ROOM_CREATED, &two_strings("ABCD", "me")).unwrap();
        assert!(created.is_host && created.state.is_none());
    }

    #[test]
    fn truncated_data_is_malformed() {
        let data = Track {
            title: "Hello".into(),
            ..Track::default()
        }
        .encode();
        assert_eq!(Track::decode(&data[..data.len() - 1]), Err(Malformed));
    }
}
