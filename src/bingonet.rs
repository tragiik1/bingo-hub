//! A small client for the EldenBingo 0.18 lobby server (bingocard.bingobrawlers.com:4501), so the
//! hub can join a race lobby as a second member of the player's team and claim squares for them
//! (the phone board, see phone.rs).
//!
//! Wire format (EldenBingo's "Neto" layer): each message is a little-endian i32 length followed
//! by one or more MessagePack values. A value is a Packet, `[type, count, typeName, object, ...]`,
//! where objects are C# records serialized "contractless" (maps keyed by property name, Guids as
//! strings, enums as ints). Large messages arrive LZ4-compressed (MessagePack-CSharp's
//! Lz4BlockArray); the server also reads uncompressed ones, so we always send plain.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SERVER: &str = "bingocard.bingobrawlers.com";
pub const PORT: u16 = 4501;
pub const VERSION: &str = "0.18.0";

// ---------------------------------------------------------------------------------------------
// MessagePack

#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Bin(Vec<u8>),
    Arr(Vec<V>),
    Map(Vec<(V, V)>),
    Ext(i8, Vec<u8>),
}

impl V {
    pub fn get(&self, key: &str) -> Option<&V> {
        match self {
            V::Map(m) => m.iter().find(|(k, _)| matches!(k, V::Str(s) if s.eq_ignore_ascii_case(key))).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn int(&self) -> Option<i64> {
        match self {
            V::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            V::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn bool(&self) -> Option<bool> {
        match self {
            V::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn arr(&self) -> &[V] {
        match self {
            V::Arr(a) => a,
            _ => &[],
        }
    }
    fn s(s: &str) -> V {
        V::Str(s.to_string())
    }
    fn map(pairs: Vec<(&str, V)>) -> V {
        V::Map(pairs.into_iter().map(|(k, v)| (V::s(k), v)).collect())
    }
}

pub fn encode(v: &V, out: &mut Vec<u8>) {
    match v {
        V::Nil => out.push(0xc0),
        V::Bool(b) => out.push(if *b { 0xc3 } else { 0xc2 }),
        V::Int(i) => {
            let i = *i;
            if (0..128).contains(&i) {
                out.push(i as u8);
            } else if (-32..0).contains(&i) {
                out.push(i as i8 as u8);
            } else {
                out.push(0xd3);
                out.extend_from_slice(&i.to_be_bytes());
            }
        }
        V::Float(f) => {
            out.push(0xcb);
            out.extend_from_slice(&f.to_be_bytes());
        }
        V::Str(s) => {
            let n = s.len();
            if n < 32 {
                out.push(0xa0 | n as u8);
            } else if n < 256 {
                out.extend_from_slice(&[0xd9, n as u8]);
            } else if n < 65536 {
                out.push(0xda);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            } else {
                out.push(0xdb);
                out.extend_from_slice(&(n as u32).to_be_bytes());
            }
            out.extend_from_slice(s.as_bytes());
        }
        V::Bin(b) => {
            out.push(0xc6);
            out.extend_from_slice(&(b.len() as u32).to_be_bytes());
            out.extend_from_slice(b);
        }
        V::Arr(a) => {
            if a.len() < 16 {
                out.push(0x90 | a.len() as u8);
            } else {
                out.push(0xdd);
                out.extend_from_slice(&(a.len() as u32).to_be_bytes());
            }
            for x in a {
                encode(x, out);
            }
        }
        V::Map(m) => {
            if m.len() < 16 {
                out.push(0x80 | m.len() as u8);
            } else {
                out.push(0xdf);
                out.extend_from_slice(&(m.len() as u32).to_be_bytes());
            }
            for (k, x) in m {
                encode(k, out);
                encode(x, out);
            }
        }
        V::Ext(t, d) => {
            out.push(0xc9);
            out.extend_from_slice(&(d.len() as u32).to_be_bytes());
            out.push(*t as u8);
            out.extend_from_slice(d);
        }
    }
}

pub struct Reader<'a> {
    b: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    pub fn done(&self) -> bool {
        self.pos >= self.b.len()
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.b.len() {
            return Err("message ended early".into());
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u(&mut self, n: usize) -> Result<u64, String> {
        Ok(self.take(n)?.iter().fold(0u64, |a, &x| (a << 8) | x as u64))
    }
    pub fn read(&mut self) -> Result<V, String> {
        let t = self.take(1)?[0];
        Ok(match t {
            0x00..=0x7f => V::Int(t as i64),
            0x80..=0x8f => self.map((t & 0x0f) as usize)?,
            0x90..=0x9f => self.arr((t & 0x0f) as usize)?,
            0xa0..=0xbf => self.string((t & 0x1f) as usize)?,
            0xc0 => V::Nil,
            0xc2 => V::Bool(false),
            0xc3 => V::Bool(true),
            0xc4 | 0xc5 | 0xc6 => {
                let n = self.u(1 << (t - 0xc4))? as usize;
                V::Bin(self.take(n)?.to_vec())
            }
            0xc7 | 0xc8 | 0xc9 => {
                let n = self.u(1 << (t - 0xc7))? as usize;
                let ty = self.take(1)?[0] as i8;
                V::Ext(ty, self.take(n)?.to_vec())
            }
            0xca => V::Float(f32::from_bits(self.u(4)? as u32) as f64),
            0xcb => V::Float(f64::from_bits(self.u(8)?)),
            0xcc..=0xcf => V::Int(self.u(1 << (t - 0xcc))? as i64),
            0xd0 => V::Int(self.take(1)?[0] as i8 as i64),
            0xd1 => V::Int(self.u(2)? as u16 as i16 as i64),
            0xd2 => V::Int(self.u(4)? as u32 as i32 as i64),
            0xd3 => V::Int(self.u(8)? as i64),
            0xd4..=0xd8 => {
                let ty = self.take(1)?[0] as i8;
                V::Ext(ty, self.take(1 << (t - 0xd4))?.to_vec())
            }
            0xd9 | 0xda | 0xdb => {
                let n = self.u(1 << (t - 0xd9))? as usize;
                self.string(n)?
            }
            0xdc | 0xdd => {
                let n = self.u(if t == 0xdc { 2 } else { 4 })? as usize;
                self.arr(n)?
            }
            0xde | 0xdf => {
                let n = self.u(if t == 0xde { 2 } else { 4 })? as usize;
                self.map(n)?
            }
            0xe0..=0xff => V::Int(t as i8 as i64),
            _ => return Err(format!("unknown MessagePack byte {t:#x}")),
        })
    }
    fn string(&mut self, n: usize) -> Result<V, String> {
        Ok(V::Str(String::from_utf8_lossy(self.take(n)?).into_owned()))
    }
    fn arr(&mut self, n: usize) -> Result<V, String> {
        let mut a = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            a.push(self.read()?);
        }
        Ok(V::Arr(a))
    }
    fn map(&mut self, n: usize) -> Result<V, String> {
        let mut m = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let k = self.read()?;
            let v = self.read()?;
            m.push((k, v));
        }
        Ok(V::Map(m))
    }
}

/// Decompresses one raw LZ4 block.
pub fn lz4_block(src: &[u8], size: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(size);
    let mut i = 0;
    let bad = || "bad LZ4 data".to_string();
    while i < src.len() {
        let token = src[i];
        i += 1;
        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            loop {
                let b = *src.get(i).ok_or_else(bad)?;
                i += 1;
                lit += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        out.extend_from_slice(src.get(i..i + lit).ok_or_else(bad)?);
        i += lit;
        if i >= src.len() {
            break;
        }
        let off = u16::from_le_bytes([src[i], *src.get(i + 1).ok_or_else(bad)?]) as usize;
        i += 2;
        let mut ml = (token & 15) as usize;
        if ml == 15 {
            loop {
                let b = *src.get(i).ok_or_else(bad)?;
                i += 1;
                ml += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        ml += 4;
        if off == 0 || off > out.len() {
            return Err(bad());
        }
        let start = out.len() - off;
        for k in 0..ml {
            let b = out[start + k];
            out.push(b);
        }
    }
    Ok(out)
}

/// Undoes MessagePack-CSharp's LZ4 wrapping, if any: Lz4BlockArray is `[ext 98 (block sizes),
/// bin, bin, ...]`, Lz4Block is `ext 99 (i32 size + block)`. Anything else is returned as is.
fn unwrap_lz4(v: V) -> Result<V, String> {
    match v {
        V::Arr(a) if matches!(a.first(), Some(V::Ext(98, _))) => {
            let V::Ext(_, sizes) = &a[0] else { unreachable!() };
            let mut r = Reader::new(sizes);
            let mut plain = Vec::new();
            for block in &a[1..] {
                let size = r.read()?.int().ok_or("bad LZ4 header")? as usize;
                let V::Bin(data) = block else { return Err("bad LZ4 block".into()) };
                plain.extend(lz4_block(data, size)?);
            }
            Reader::new(&plain).read()
        }
        V::Ext(99, d) => {
            let mut r = Reader::new(&d);
            let size = r.read()?.int().ok_or("bad LZ4 header")? as usize;
            let plain = lz4_block(&d[r.pos..], size)?;
            Reader::new(&plain).read()
        }
        v => Ok(v),
    }
}

// ---------------------------------------------------------------------------------------------
// Packets

const P_REGISTER_ACCEPTED: i64 = 0;
const P_REGISTER_DENIED: i64 = 1;
const P_CLIENT_DROPPED: i64 = 2;
const P_SHUTDOWN: i64 = 3;
const P_CLIENT_REGISTER: i64 = 4;
const P_CLIENT_DISCONNECT: i64 = 5;
const P_OBJECTS: i64 = 6;

fn packet(kind: i64, objects: Vec<(&str, V)>) -> Vec<u8> {
    let mut a = vec![V::Int(kind), V::Int(objects.len() as i64)];
    for (name, o) in objects {
        a.push(V::s(name));
        a.push(o);
    }
    let mut body = Vec::new();
    encode(&V::Arr(a), &mut body);
    let mut msg = (body.len() as i32).to_le_bytes().to_vec();
    msg.extend(body);
    msg
}

/// Splits one received message into packets: (type, [(typeName, object)]).
pub fn parse_message(bytes: &[u8]) -> Result<Vec<(i64, Vec<(String, V)>)>, String> {
    let mut r = Reader::new(bytes);
    let mut out = Vec::new();
    while !r.done() {
        // Big packets are compressed as a whole, and big objects inside them on their own too.
        let v = unwrap_lz4(r.read()?)?;
        let a = v.arr();
        if a.len() < 2 {
            return Err("bad packet".into());
        }
        let kind = a[0].int().unwrap_or(-1);
        // Each object is serialized (and compressed) on its own.
        let mut objs = Vec::new();
        for c in a[2..].chunks(2).filter(|c| c.len() == 2) {
            objs.push((c[0].str().unwrap_or("").to_string(), unwrap_lz4(c[1].clone())?));
        }
        out.push((kind, objs));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Lobby state

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct User {
    pub guid: String,
    pub nick: String,
    pub team: i64,
    pub admin: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Square {
    pub text: String,
    pub teams: Vec<i64>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Score {
    pub team: i64,
    pub name: String,
    pub score: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub enum Conn {
    Connecting,
    /// Connected and in the lobby.
    Joined,
    Failed(String),
    Closed,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Lobby {
    pub conn: Conn,
    pub room: String,
    pub me: String,
    pub users: Vec<User>,
    pub size: usize,
    pub lockout: bool,
    pub squares: Vec<Square>,
    pub scores: Vec<Score>,
    /// 0 not running, 1 starting, 2 preparation, 3 running, 4 finished.
    pub status: i64,
    pub paused: bool,
    /// Match clock in ms when `clock_at` was set (negative during the countdown).
    pub timer: i64,
    #[serde(skip)]
    pub clock_at: Option<Instant>,
    /// The last message from the server worth showing (admin notices, errors).
    pub notice: String,
}

impl Lobby {
    fn new() -> Self {
        Lobby { conn: Conn::Connecting, room: String::new(), me: String::new(), users: vec![], size: 0, lockout: true, squares: vec![], scores: vec![], status: 0, paused: false, timer: 0, clock_at: None, notice: String::new() }
    }
    /// The match clock right now, in ms.
    pub fn clock(&self) -> i64 {
        let running = (1..4).contains(&self.status) && !self.paused;
        self.timer + if running { self.clock_at.map_or(0, |t| t.elapsed().as_millis() as i64) } else { 0 }
    }
    pub fn my_team(&self) -> Option<i64> {
        self.users.iter().find(|u| u.guid == self.me).map(|u| u.team)
    }
    pub fn team_name(&self, team: i64) -> String {
        self.scores.iter().find(|s| s.team == team).map(|s| s.name.clone()).unwrap_or_else(|| TEAM_NAMES.get(team as usize).unwrap_or(&"?").to_string())
    }
}

/// EldenBingo's team colours, in team order (BingoConstants.TeamColors).
pub const TEAM_NAMES: [&str; 10] = ["Red", "Blue", "Green", "Yellow", "Orange", "Purple", "Cyan", "Pink", "Brown", "Gray"];
pub const TEAM_RGB: [(u8, u8, u8); 10] = [(225, 45, 45), (45, 95, 225), (40, 170, 60), (220, 200, 30), (240, 130, 20), (150, 60, 200), (30, 190, 200), (240, 110, 190), (130, 85, 45), (128, 128, 128)];

fn user(v: &V) -> User {
    User {
        guid: v.get("Guid").and_then(V::str).unwrap_or("").to_lowercase(),
        nick: v.get("Nick").and_then(V::str).unwrap_or("").to_string(),
        team: v.get("Team").and_then(V::int).unwrap_or(-1),
        admin: v.get("IsAdmin").and_then(V::bool).unwrap_or(false),
    }
}

fn square(v: &V) -> Square {
    Square { text: v.get("Text").and_then(V::str).unwrap_or("").to_string(), teams: v.get("Team").map(|t| t.arr().iter().filter_map(V::int).collect()).unwrap_or_default() }
}

fn apply(l: &mut Lobby, name: &str, o: &V) {
    match name {
        "ServerJoinRoomAccepted" => {
            l.conn = Conn::Joined;
            l.room = o.get("RoomName").and_then(V::str).unwrap_or("").to_string();
            l.users = o.get("Users").map(|u| u.arr().iter().map(user).collect()).unwrap_or_default();
            set_status(l, o);
        }
        "ServerJoinRoomDenied" | "ServerCreateRoomDenied" => {
            l.conn = Conn::Failed(o.get("Reason").and_then(V::str).unwrap_or("Couldn't join the lobby").to_string());
        }
        "ServerUserJoinedRoom" => {
            if let Some(u) = o.get("User") {
                let u = user(u);
                l.users.retain(|x| x.guid != u.guid);
                l.users.push(u);
            }
        }
        "ServerUserLeftRoom" => {
            if let Some(u) = o.get("User") {
                let g = user(u).guid;
                l.users.retain(|x| x.guid != g);
            }
        }
        "ServerUserBannedFromRoom" => {
            if let Some(u) = o.get("User") {
                let g = user(u).guid;
                if g == l.me {
                    l.conn = Conn::Failed("Removed from the lobby by an admin".into());
                }
                l.users.retain(|x| x.guid != g);
            }
        }
        "ServerUserChangedTeam" => {
            if let Some(us) = o.get("Users") {
                l.users = us.arr().iter().map(user).collect();
            }
        }
        "ServerMatchStatusUpdate" => set_status(l, o),
        "ServerEntireBingoBoardUpdate" => {
            l.size = o.get("Size").and_then(V::int).unwrap_or(0).max(0) as usize;
            l.lockout = o.get("Lockout").and_then(V::bool).unwrap_or(true);
            l.squares = o.get("Squares").map(|s| s.arr().iter().map(square).collect()).unwrap_or_default();
            if l.squares.len() != l.size * l.size {
                l.size = 0;
                l.squares.clear();
            }
        }
        "ServerSquareUpdate" => {
            let i = o.get("Index").and_then(V::int).unwrap_or(-1);
            if let (Some(s), Some(sq)) = (o.get("Square"), l.squares.get_mut(i.max(0) as usize)) {
                if i >= 0 {
                    *sq = square(s);
                }
            }
        }
        "ServerUserChecked" => {
            let i = o.get("Index").and_then(V::int).unwrap_or(-1);
            if let (true, Some(sq)) = (i >= 0, l.squares.get_mut(i.max(0) as usize)) {
                sq.teams = o.get("TeamsChecked").map(|t| t.arr().iter().filter_map(V::int).collect()).unwrap_or_default();
            }
        }
        "ServerScoreboardUpdate" => {
            l.scores = o
                .get("Scoreboard")
                .map(|s| s.arr().iter().map(|t| Score { team: t.get("Team").and_then(V::int).unwrap_or(-1), name: t.get("Name").and_then(V::str).unwrap_or("").to_string(), score: t.get("Score").and_then(V::int).unwrap_or(0) }).collect())
                .unwrap_or_default();
        }
        "ServerAdminStatusMessage" | "ServerBroadcastMessage" | "ServerToClientErrorMessage" => {
            l.notice = o.get("Message").and_then(V::str).unwrap_or("").to_string();
        }
        _ => {}
    }
}

fn set_status(l: &mut Lobby, o: &V) {
    l.status = o.get("MatchStatus").and_then(V::int).unwrap_or(0);
    l.paused = o.get("Paused").and_then(V::bool).unwrap_or(false);
    l.timer = o.get("Timer").and_then(V::int).unwrap_or(0);
    l.clock_at = Some(Instant::now());
}

// ---------------------------------------------------------------------------------------------
// Connection

pub enum Cmd {
    /// Claim (or unclaim) a square for our own team.
    Check(usize),
    /// Raw objects (tests use it to run a lobby).
    #[cfg_attr(not(test), allow(dead_code))]
    Send(Vec<(String, V)>),
    Leave,
}

#[derive(Clone)]
pub struct Client {
    pub lobby: Arc<Mutex<Lobby>>,
    tx: Sender<Cmd>,
}

pub struct Join {
    pub room: String,
    pub nick: String,
    pub team: i64,
    /// Join an existing lobby (false creates one; tests only).
    pub create: Option<V>,
    /// Kept per hub so a reconnect replaces our old connection instead of adding a second one,
    /// and different from EldenBingo's own token so we never replace the player's real client.
    pub token: String,
    /// When set, switch to the team of the lobby member with this nick once we're in.
    pub follow: Option<String>,
}

impl Client {
    pub fn check(&self, i: usize) {
        let _ = self.tx.send(Cmd::Check(i));
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn send(&self, objects: Vec<(&str, V)>) {
        let _ = self.tx.send(Cmd::Send(objects.into_iter().map(|(n, v)| (n.to_string(), v)).collect()));
    }
    pub fn leave(&self) {
        let _ = self.tx.send(Cmd::Leave);
    }
    pub fn snapshot(&self) -> Lobby {
        self.lobby.lock().unwrap().clone()
    }

    pub fn connect(join: Join) -> Client {
        let lobby = Arc::new(Mutex::new(Lobby::new()));
        let (tx, rx) = channel();
        let l2 = lobby.clone();
        std::thread::spawn(move || {
            let r = run(&l2, join, rx);
            let mut l = l2.lock().unwrap();
            match r {
                Err(e) if !matches!(l.conn, Conn::Failed(_)) => l.conn = Conn::Failed(e),
                _ if matches!(l.conn, Conn::Failed(_)) => {}
                _ => l.conn = Conn::Closed,
            }
        });
        Client { lobby, tx }
    }
}

fn read_message(s: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut n = [0u8; 4];
    s.read_exact(&mut n)?;
    let n = i32::from_le_bytes(n);
    if !(0..=64 << 20).contains(&n) {
        return Err(std::io::Error::other("bad message length"));
    }
    let mut b = vec![0; n as usize];
    s.read_exact(&mut b)?;
    Ok(b)
}

fn run(lobby: &Arc<Mutex<Lobby>>, join: Join, rx: Receiver<Cmd>) -> Result<(), String> {
    let addr = (SERVER, PORT).to_socket_addrs().map_err(|e| format!("Couldn't find the bingo server: {e}"))?.find(|a| a.is_ipv4()).ok_or("Couldn't find the bingo server")?;
    let s = TcpStream::connect_timeout(&addr, Duration::from_secs(8)).map_err(|e| format!("Couldn't reach the bingo server: {e}"))?;
    s.set_nodelay(true).ok();
    let mut w = s.try_clone().map_err(|e| e.to_string())?;
    w.write_all(&packet(P_CLIENT_REGISTER, vec![("Neto.Shared.ClientRegister", V::map(vec![("Message", V::s("hello")), ("Version", V::s(VERSION)), ("IdentityToken", V::s(&join.token))]))]))
        .map_err(|e| e.to_string())?;

    // Reader thread: parse messages into the shared lobby, wake the writer on changes.
    let (evt_tx, evt_rx) = channel::<Result<(), String>>();
    let l2 = lobby.clone();
    let mut rs = s.try_clone().map_err(|e| e.to_string())?;
    std::thread::spawn(move || loop {
        let msg = match read_message(&mut rs) {
            Ok(m) => m,
            Err(e) => {
                let _ = evt_tx.send(Err(format!("Lost the connection to the bingo server ({e})")));
                return;
            }
        };
        if cfg!(test) && std::env::var_os("BINGONET_DEBUG").is_some() {
            eprintln!("<< frame {} bytes: {:?}", msg.len(), &msg[..msg.len().min(40)]);
        }
        let packets = match parse_message(&msg) {
            Ok(p) => p,
            Err(e) => {
                let _ = evt_tx.send(Err(format!("Unexpected data from the bingo server: {e}")));
                return;
            }
        };
        for (kind, objs) in packets {
            if cfg!(test) && std::env::var_os("BINGONET_DEBUG").is_some() {
                for (n, o) in &objs {
                    eprintln!("<< {kind} {n} {}", format!("{o:?}").chars().take(600).collect::<String>());
                }
            }
            let mut l = l2.lock().unwrap();
            match kind {
                P_REGISTER_ACCEPTED => {
                    if let Some((_, o)) = objs.first() {
                        l.me = o.get("ClientGuid").and_then(V::str).unwrap_or("").to_lowercase();
                    }
                    drop(l);
                    let _ = evt_tx.send(Ok(()));
                }
                P_REGISTER_DENIED => {
                    let why = objs.first().and_then(|(_, o)| o.get("Message")).and_then(V::str).unwrap_or("refused").to_string();
                    l.conn = Conn::Failed(format!("The bingo server refused the connection: {why}"));
                    drop(l);
                    let _ = evt_tx.send(Err(String::new()));
                    return;
                }
                P_CLIENT_DROPPED | P_SHUTDOWN => {
                    let why = objs.first().and_then(|(_, o)| o.get("Reason")).and_then(V::str).unwrap_or("server closed the connection").to_string();
                    l.conn = Conn::Failed(format!("Disconnected: {why}"));
                    drop(l);
                    let _ = evt_tx.send(Err(String::new()));
                    return;
                }
                P_OBJECTS => {
                    for (name, o) in &objs {
                        apply(&mut l, name, o);
                    }
                }
                _ => {}
            }
        }
    });

    let send = |w: &mut TcpStream, objs: Vec<(&str, V)>| w.write_all(&packet(P_OBJECTS, objs)).map_err(|e| e.to_string());
    let deadline = Instant::now() + Duration::from_secs(10);
    // Wait for registration.
    match evt_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("The bingo server didn't answer".into()),
    }
    let nick = V::s(&join.nick);
    match &join.create {
        Some(settings) => send(&mut w, vec![("ClientRequestCreateRoom", V::map(vec![("RoomName", V::s(&join.room)), ("AdminPass", V::s("")), ("Nick", nick), ("Team", V::Int(join.team)), ("Settings", settings.clone())]))])?,
        None => send(&mut w, vec![("ClientRequestJoinRoom", V::map(vec![("RoomName", V::s(&join.room)), ("AdminPass", V::s("")), ("Nick", nick), ("Team", V::Int(join.team))]))])?,
    }
    let mut followed = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Cmd::Check(i)) => {
                let me = lobby.lock().unwrap().me.clone();
                send(&mut w, vec![("ClientTryCheck", V::map(vec![("Index", V::Int(i as i64)), ("ForUser", V::s(&me))]))])?;
            }
            Ok(Cmd::Send(objs)) => send(&mut w, objs.iter().map(|(n, v)| (n.as_str(), v.clone())).collect())?,
            Ok(Cmd::Leave) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
        if let Ok(Err(e)) = evt_rx.try_recv() {
            return Err(e);
        }
        // Follow the player's team, so the phone always claims for whichever team they're on.
        if let Some(who) = &join.follow {
            let l = lobby.lock().unwrap();
            if matches!(l.conn, Conn::Joined) {
                let target = l.users.iter().find(|u| u.guid != l.me && u.nick.eq_ignore_ascii_case(who)).map(|u| u.team);
                if let (Some(t), Some(mine)) = (target, l.my_team()) {
                    if t != mine && t >= 0 && !followed {
                        drop(l);
                        send(&mut w, vec![("ClientRequestTeamChange", V::map(vec![("Team", V::Int(t))]))])?;
                        followed = true;
                    } else if t == mine {
                        followed = false;
                    }
                }
            }
        }
    }
    let _ = send(&mut w, vec![("ClientRequestLeaveRoom", V::Map(vec![]))]);
    let _ = w.write_all(&packet(P_CLIENT_DISCONNECT, vec![]));
    let _ = s.shutdown(Shutdown::Both);
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn msgpack_round_trip() {
        let v = V::Arr(vec![V::Int(6), V::Int(-5), V::Int(300), V::Int(-70000), V::s("ClientTryCheck"), V::map(vec![("Index", V::Int(12)), ("ok", V::Bool(true)), ("s", V::s(&"x".repeat(300)))])]);
        let mut b = Vec::new();
        encode(&v, &mut b);
        assert_eq!(Reader::new(&b).read().unwrap(), v);
    }

    #[test]
    fn lz4_literals_and_matches() {
        // "abcabcabcabc": 3 literals then a match of 9 at offset 3.
        let block = [0x35, b'a', b'b', b'c', 3, 0];
        assert_eq!(lz4_block(&block, 12).unwrap(), b"abcabcabcabc");
    }

    /// Joins a throwaway lobby on the real server with two connections (an admin that uploads a
    /// board and starts the match, and a "phone" on the same team) and checks a claim from the
    /// phone shows up for the admin. Run with `cargo test live_lobby -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_lobby() {
        let tag = format!("hubtest-{}", std::process::id());
        let admin = running_lobby(&tag, &(0..25).map(|i| format!("Test square {i}")).collect::<Vec<_>>());
        let phone = Client::connect(Join { room: tag.clone(), nick: "hubtest-admin (phone)".into(), team: 1, create: None, token: format!("{tag}p"), follow: Some("hubtest-admin".into()) });
        wait(&phone, |l| matches!(l.conn, Conn::Joined) && l.squares.len() == 25, "phone joins and sees the board");
        wait(&phone, |l| l.my_team() == Some(0), "phone follows the admin's team");
        phone.check(7);
        wait(&admin, |l| l.squares[7].teams == vec![0], "claim reaches the admin");
        phone.check(7);
        wait(&admin, |l| l.squares[7].teams.is_empty(), "unclaim reaches the admin");
        let snap = phone.snapshot();
        println!("users: {:?}\nclock {} ms, status {}", snap.users, snap.clock(), snap.status);
        phone.leave();
        admin.leave();
        std::thread::sleep(Duration::from_millis(500));
    }

    /// Creates lobby `tag` as "hubtest-admin" (red team), uploads `goals` and starts the match.
    pub fn running_lobby(tag: &str, goals: &[String]) -> Client {
        let settings = V::map(vec![
            ("BoardSize", V::Int(5)),
            ("Lockout", V::Bool(true)),
            ("RandomClasses", V::Bool(false)),
            ("ValidClasses", V::Arr(vec![V::Int(0)])),
            ("NumberOfClasses", V::Int(1)),
            ("CategoryLimit", V::Int(0)),
            ("RandomSeed", V::Int(0)),
            ("PreparationTime", V::Int(0)),
            ("PointsPerBingoLine", V::Int(0)),
        ]);
        let admin = Client::connect(Join { room: tag.to_string(), nick: "hubtest-admin".into(), team: 0, create: Some(settings), token: format!("{tag}a"), follow: None });
        wait(&admin, |l| matches!(l.conn, Conn::Joined), "admin joins");
        let json = serde_json::to_string(&goals.iter().map(|g| serde_json::json!({ "name": g })).collect::<Vec<_>>()).unwrap();
        admin.send(vec![("ClientBingoJson", V::map(vec![("Json", V::s(&json))]))]);
        std::thread::sleep(Duration::from_millis(800));
        admin.send(vec![("ClientChangeMatchStatus", V::map(vec![("MatchStatus", V::Int(1))]))]);
        wait(&admin, |l| l.status == 3 && l.squares.len() == 25, "match running");
        admin
    }
    pub fn wait(c: &Client, ok: impl Fn(&Lobby) -> bool, what: &str) {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(20) {
            let l = c.snapshot();
            if let Conn::Failed(e) = &l.conn {
                panic!("{what}: {e}");
            }
            if ok(&l) {
                println!("ok: {what}");
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("timed out: {what} ({:?})", c.snapshot());
    }
}
