//! Phone board: the hub joins the race lobby as "<nick> (phone)" on the player's team (see
//! bingonet.rs) and serves the board as a small web page on the local network, so a player without
//! a second monitor can see and claim squares from their phone. The page lives at a random,
//! unguessable path, so nobody else on the Wi-Fi can claim squares for them.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::bingonet::{Client, Conn, Join, TEAM_NAMES, TEAM_RGB};

pub struct Phone {
    pub client: Client,
    pub url: String,
    pub qr: Option<qrcodegen::QrCode>,
    /// Set when the local web server couldn't start.
    pub serve_error: Option<String>,
    stop: Arc<AtomicBool>,
}

impl Drop for Phone {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.client.leave();
    }
}

/// Fixed so the phone's bookmark keeps working between races (falls back to any free port).
const PORT: u16 = 47130;

/// The player's EldenBingo nickname and default team, from its saved settings.
pub fn eldenbingo_player() -> Option<(String, i64)> {
    let base = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("EldenBingo");
    let mut configs: Vec<_> = std::fs::read_dir(&base)
        .ok()?
        .flatten()
        .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten())
        .map(|v| v.path().join("user.config"))
        .filter(|p| p.is_file())
        .collect();
    // The hub ships EldenBingo 0.18, so its settings win over leftovers from other versions.
    configs.sort_by_key(|p| (p.to_string_lossy().contains("0.18."), std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    let text = std::fs::read_to_string(configs.last()?).ok()?;
    let value = |name: &str| -> Option<String> {
        let at = text.find(&format!("name=\"{name}\""))?;
        let rest = &text[at..];
        let v = &rest[rest.find("<value>")? + 7..];
        Some(v[..v.find("</value>")?].trim().to_string())
    };
    let nick = value("Nickname").filter(|n| !n.is_empty())?;
    let team = value("Team").and_then(|t| t.parse().ok()).unwrap_or(0);
    Some((xml_unescape(&nick), team))
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

pub fn random_token(n: usize) -> String {
    use std::hash::{BuildHasher, Hasher};
    const ABC: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut out = String::new();
    let mut i = 0u64;
    while out.len() < n {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i ^ std::process::id() as u64);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let mut x = h.finish();
        for _ in 0..8 {
            out.push(ABC[(x % ABC.len() as u64) as usize] as char);
            x /= ABC.len() as u64;
        }
        i += 1;
    }
    out.truncate(n);
    out
}

/// This PC's address on the local network (no packets are sent).
fn lan_ip() -> String {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.0.2.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

impl Phone {
    /// `identity` is the hub's own EldenBingo identity (kept in settings), so reconnecting
    /// replaces our previous phone connection rather than adding another.
    pub fn start(lobby: &str, player: &str, team: i64, identity: &str) -> Phone {
        let client = Client::connect(Join {
            room: lobby.trim().to_string(),
            nick: format!("{player} (phone)"),
            team,
            create: None,
            token: identity.to_string(),
            follow: Some(player.to_string()),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let key = random_token(10);
        let (url, serve_error) = match TcpListener::bind(("0.0.0.0", PORT)).or_else(|_| TcpListener::bind(("0.0.0.0", 0))) {
            Ok(l) => {
                let port = l.local_addr().map(|a| a.port()).unwrap_or(PORT);
                let (c, s, k) = (client.clone(), stop.clone(), key.clone());
                std::thread::spawn(move || serve(l, c, k, s));
                (format!("http://{}:{port}/{key}", lan_ip()), None)
            }
            Err(e) => (String::new(), Some(format!("Couldn't open the phone page: {e}"))),
        };
        let qr = (!url.is_empty()).then(|| qrcodegen::QrCode::encode_text(&url, qrcodegen::QrCodeEcc::Medium).ok()).flatten();
        Phone { client, url, qr, serve_error, stop }
    }
}

fn serve(listener: TcpListener, client: Client, key: String, stop: Arc<AtomicBool>) {
    let _ = listener.set_nonblocking(true);
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((s, _)) => {
                let (c, k) = (client.clone(), key.clone());
                std::thread::spawn(move || {
                    let _ = handle(s, &c, &k);
                });
            }
            Err(_) => std::thread::sleep(Duration::from_millis(40)),
        }
    }
}

fn handle(mut s: TcpStream, client: &Client, key: &str) -> std::io::Result<()> {
    s.set_nonblocking(false)?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        let n = s.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf);
    let mut parts = head.lines().next().unwrap_or("").split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let base = format!("/{key}");
    let (status, ctype, body): (&str, &str, Vec<u8>) = match (method, path.strip_prefix(&base)) {
        ("GET", Some("") | Some("/")) => ("200 OK", "text/html; charset=utf-8", PAGE.as_bytes().to_vec()),
        ("GET", Some("/state")) => ("200 OK", "application/json", state_json(client).into_bytes()),
        ("POST", Some(rest)) if rest.starts_with("/check/") => match rest[7..].parse::<usize>() {
            Ok(i) => {
                client.check(i);
                ("200 OK", "application/json", b"{}".to_vec())
            }
            Err(_) => ("400 Bad Request", "text/plain", b"bad square".to_vec()),
        },
        _ => ("404 Not Found", "text/plain", b"Not found. Scan the QR code in Bingo Hub again.".to_vec()),
    };
    let header = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    s.write_all(header.as_bytes())?;
    s.write_all(&body)?;
    s.flush()
}

fn state_json(client: &Client) -> String {
    let l = client.snapshot();
    let my = l.my_team();
    let (conn, error) = match &l.conn {
        Conn::Connecting => ("connecting", String::new()),
        Conn::Joined => ("joined", String::new()),
        Conn::Failed(e) => ("failed", e.clone()),
        Conn::Closed => ("closed", "Phone board stopped in Bingo Hub.".to_string()),
    };
    let color = |t: i64| TEAM_RGB.get(t.max(0) as usize).map(|(r, g, b)| format!("rgb({r},{g},{b})")).unwrap_or_else(|| "rgb(128,128,128)".into());
    serde_json::json!({
        "conn": conn,
        "error": error,
        "room": l.room,
        "status": l.status,
        "paused": l.paused,
        "clock": l.clock(),
        "size": l.size,
        "lockout": l.lockout,
        "myTeam": my,
        "myName": my.map(|t| l.team_name(t)),
        "myColor": my.map(color),
        "squares": l.squares.iter().map(|s| serde_json::json!({"text": s.text, "teams": s.teams})).collect::<Vec<_>>(),
        "teams": (0..TEAM_NAMES.len() as i64).map(|t| serde_json::json!({"name": l.team_name(t), "color": color(t)})).collect::<Vec<_>>(),
        "scores": l.scores.iter().map(|s| serde_json::json!({"team": s.team, "name": s.name, "score": s.score, "color": color(s.team)})).collect::<Vec<_>>(),
        "notice": l.notice,
    })
    .to_string()
}

const PAGE: &str = r##"<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover">
<meta name="theme-color" content="#141418">
<title>Bingo board</title>
<style>
:root{--bg:#141418;--surface:#1d1d23;--raised:#26262e;--border:#33333d;--text:#ececf1;--muted:#9a9aa6;--accent:#d8b45c}
*{box-sizing:border-box;-webkit-tap-highlight-color:transparent}
[hidden]{display:none!important}
html,body{margin:0;height:100%;background:var(--bg);color:var(--text);font:15px/1.3 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}
body{display:flex;flex-direction:column;padding:max(10px,env(safe-area-inset-top)) 10px max(10px,env(safe-area-inset-bottom))}
header{display:flex;align-items:center;gap:10px;margin-bottom:8px;min-height:34px}
#team{width:14px;height:14px;border-radius:4px;background:var(--raised);flex:none}
#who{flex:1;min-width:0}
#who b{display:block;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
#who span{color:var(--muted);font-size:12px}
#clock{font-variant-numeric:tabular-nums;font-weight:600;font-size:20px}
#status{font-size:12px;color:var(--muted);text-align:right}
#scores{display:flex;gap:6px;flex-wrap:wrap;margin-bottom:8px}
.score{display:flex;align-items:center;gap:6px;background:var(--surface);border:1px solid var(--border);border-radius:999px;padding:3px 10px 3px 6px;font-size:13px}
.score i{width:10px;height:10px;border-radius:3px;display:block}
#board{flex:1;display:grid;gap:5px;min-height:0}
.sq{position:relative;border-radius:8px;background:var(--surface);border:1px solid var(--border);display:flex;align-items:center;justify-content:center;text-align:center;padding:4px;overflow:hidden;user-select:none;font-size:var(--fs,12px);line-height:1.15;transition:transform .08s}
.sq:active{transform:scale(.96)}
.sq.mine{border-color:#fff;color:#fff;font-weight:600}
.sq.other{opacity:.55}
.sq.pending{outline:2px dashed var(--accent);outline-offset:-4px}
.sq span{position:relative;z-index:1;text-shadow:0 1px 2px rgba(0,0,0,.6)}
#msg{flex:1;display:flex;align-items:center;justify-content:center;text-align:center;color:var(--muted);padding:20px}
#notice{color:var(--muted);font-size:12px;margin-top:6px;min-height:15px}
</style></head><body>
<header><div id="team"></div><div id="who"><b id="room">Connecting…</b><span id="sub"></span></div><div><div id="clock"></div><div id="status"></div></div></header>
<div id="scores"></div>
<div id="board" hidden></div>
<div id="msg">Connecting to the lobby…</div>
<div id="notice"></div>
<script>
const base = location.pathname.replace(/\/$/, '');
const $ = id => document.getElementById(id);
let st = null, got = 0, pending = new Map(), built = '';
const STATUS = ['Waiting for the race', 'Starting', 'Preparation', 'Running', 'Finished'];
function fmt(ms){const neg=ms<0;let s=Math.floor(Math.abs(ms)/1000);const h=Math.floor(s/3600);s%=3600;const m=Math.floor(s/60);s%=60;return (neg?'-':'')+(h?h+':'+String(m).padStart(2,'0'):m)+':'+String(s).padStart(2,'0')}
async function poll(){
  try{const r=await fetch(base+'/state',{cache:'no-store'});if(!r.ok)throw 0;st=await r.json();got=performance.now();render()}
  catch(e){$('msg').hidden=false;$('board').hidden=true;$('msg').textContent='Can\'t reach Bingo Hub. Is your phone on the same Wi-Fi as the PC, and the phone board still on?'}
  setTimeout(poll,700)
}
function render(){
  const s=st;
  $('room').textContent=s.room?('Lobby '+s.room):'Connecting…';
  $('sub').textContent=s.myName?('Claiming for '+s.myName):'';
  $('team').style.background=s.myColor||'';
  $('status').textContent=s.paused?'Paused':(STATUS[s.status]||'');
  $('scores').innerHTML=s.scores.map(t=>`<div class="score"><i style="background:${t.color}"></i>${esc(t.name)} <b>${t.score}</b></div>`).join('');
  $('notice').textContent=s.notice||'';
  let msg='';
  if(s.conn==='failed'||s.conn==='closed')msg=s.error;
  else if(s.conn==='connecting')msg='Connecting to the lobby…';
  else if(!s.size)msg=s.status<2?'The board shows here when the race starts.':'Waiting for the board…';
  $('msg').hidden=!msg;$('msg').textContent=msg;$('board').hidden=!!msg;
  if(msg)return;
  const n=s.size, b=$('board');
  const key=n+'|'+s.squares.map(q=>q.text).join('|');
  if(key!==built){
    built=key;b.innerHTML='';
    b.style.gridTemplateColumns=`repeat(${n},1fr)`;b.style.gridTemplateRows=`repeat(${n},1fr)`;
    s.squares.forEach((q,i)=>{const d=document.createElement('div');d.className='sq';d.innerHTML='<span></span>';d.firstChild.textContent=q.text;d.onclick=()=>tap(i);b.appendChild(d)});
  }
  fit();
  [...b.children].forEach((d,i)=>{
    const q=s.squares[i];const teams=q.teams||[];
    const mine=s.myTeam!=null&&teams.includes(s.myTeam);
    const p=pending.get(i);if(p!==undefined&&p!==mine)d.classList.add('pending');else{pending.delete(i);d.classList.remove('pending')}
    d.classList.toggle('mine',mine);
    d.classList.toggle('other',!mine&&teams.length>0&&s.lockout);
    const cols=teams.map(t=>(s.teams[t]||{}).color||'#888');
    d.style.background=cols.length?(cols.length===1?cols[0]:`linear-gradient(135deg,${cols.map((c,k)=>c+' '+(k*100/cols.length)+'%,'+c+' '+((k+1)*100/cols.length)+'%').join(',')})`):'';
  });
}
function fit(){const b=$('board'),n=st.size||5;if(!b.clientWidth)return;const fs=Math.max(9,Math.min(17,b.clientWidth/n/6.4,b.clientHeight/n/4.2))+'px';if(fit.last!==fs){fit.last=fs;document.documentElement.style.setProperty('--fs',fs)}}
addEventListener('resize',()=>st&&st.size&&fit());
async function tap(i){
  const s=st;if(!s||s.status!==3||s.paused)return;
  const teams=s.squares[i].teams||[];const mine=teams.includes(s.myTeam);
  if(!mine&&s.lockout&&teams.length)return;
  pending.set(i,!mine);render();
  if(navigator.vibrate)navigator.vibrate(15);
  try{await fetch(base+'/check/'+i,{method:'POST'})}catch(e){}
}
function esc(t){return String(t).replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]))}
setInterval(()=>{if(st&&st.size!==undefined){const run=st.status>=1&&st.status<=3&&!st.paused;$('clock').textContent=st.status===0?'':fmt(st.clock+(run?performance.now()-got:0))}},250);
poll();
</script></body></html>"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_url_safe() {
        let a = random_token(10);
        let b = random_token(10);
        assert_eq!(a.len(), 10);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    fn http(url: &str, method: &str) -> (String, String) {
        let rest = url.strip_prefix("http://").unwrap();
        let (host, path) = rest.split_at(rest.find('/').unwrap());
        let mut s = TcpStream::connect(host).unwrap();
        write!(s, "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: 0\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let (head, body) = out.split_once("\r\n\r\n").unwrap();
        (head.lines().next().unwrap().to_string(), body.to_string())
    }

    /// The whole phone board against the real server: a throwaway lobby, the hub's phone
    /// connection, the page, its state and a claim made through it. PHONE_HOLD=secs keeps it
    /// open afterwards to look at in a browser. `cargo test live_phone -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_phone() {
        use crate::bingonet::tests::{running_lobby, wait};
        let tag = format!("hubtest-phone-{}", std::process::id());
        let goals: Vec<String> = [
            "Kill Margit", "Acquire 3 Larval Tears", "Kill 2 Night's Cavalry", "Smithing-stone +12 weapon", "Kill a Godskin Apostle",
            "Complete 4 evergaols", "Kill Rennala", "Acquire 8 different Talismans", "Kill a Black Knife Assassin", "Rune Level 60",
            "Kill 3 Erdtree Avatars", "Kill Starscourge Radahn", "Acquire 5 map fragments", "Kill Loretta (Blue Spirit)", "Kill a Bell Bearing Hunter",
            "Drain the water in the Church District", "Kill Mohg, Lord of Blood", "Collect 4 perfumer bottles", "Kill 2 Duelist bosses", "Acquire 2 Blessings of Marika",
            "Kill a boss on Torrent the whole fight", "Kill Scaly Misbegotten", "Complete 2 cave or grotto dungeons", "Kill Leonine Misbegotten", "Acquire Moon of Nokstella",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let admin = running_lobby(&tag, &goals);
        let ph = Phone::start(&tag, "hubtest-admin", 1, &format!("{tag}hub"));
        assert!(ph.serve_error.is_none(), "{:?}", ph.serve_error);
        assert!(ph.qr.is_some());
        wait(&ph.client, |l| l.my_team() == Some(0) && l.squares.len() == 25, "phone joins on the admin's team");
        let local = format!("http://127.0.0.1:{}", ph.url.splitn(4, '/').nth(2).unwrap().rsplit(':').next().unwrap()) + "/" + ph.url.rsplit('/').next().unwrap();
        let (st, page) = http(&local, "GET");
        assert!(st.contains("200") && page.contains("<title>Bingo board</title>"), "{st}");
        let (_, json) = http(&format!("{local}/state"), "GET");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["conn"], "joined");
        assert_eq!(v["squares"].as_array().unwrap().len(), 25);
        assert_eq!(v["myTeam"], 0);
        let (st, _) = http(&format!("{local}/check/4"), "POST");
        assert!(st.contains("200"));
        wait(&admin, |l| l.squares[4].teams == vec![0], "claim from the page reaches the lobby");
        let wrong = local.rsplit_once('/').unwrap().0.to_string() + "/notthekey/state";
        assert!(http(&wrong, "GET").0.contains("404"), "other paths are refused");
        println!("phone page: {} (local {local})", ph.url);
        if let Some(secs) = std::env::var("PHONE_HOLD").ok().and_then(|s| s.parse::<u64>().ok()) {
            std::thread::sleep(Duration::from_secs(secs));
        }
        drop(ph);
        admin.leave();
        std::thread::sleep(Duration::from_millis(500));
    }
}