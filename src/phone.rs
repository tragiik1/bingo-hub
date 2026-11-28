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
    /// `key` is the secret part of the page's link; keeping it the same keeps the phone's bookmark working.
    pub fn start(lobby: &str, player: &str, team: i64, identity: &str, key: &str) -> Phone {
        let client = Client::connect(Join {
            room: lobby.trim().to_string(),
            nick: format!("{player} (phone)"),
            team,
            create: None,
            token: identity.to_string(),
            follow: Some(player.to_string()),
            retry: true,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let key = key.to_string();
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
        ("GET", Some("") | Some("/")) => ("200 OK", "text/html; charset=utf-8", page().as_bytes().to_vec()),
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

/// The page with the keep-awake clip inlined (Safari asks for byte ranges, which we don't serve).
fn page() -> &'static str {
    static PAGE_FULL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PAGE_FULL.get_or_init(|| PAGE.replace("__AWAKE__", &format!("data:video/mp4;base64,{}", base64(include_bytes!("../assets/keep-awake.mp4")))))
}

fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

fn state_json(client: &Client) -> String {
    let l = client.snapshot();
    let my = l.my_team();
    let (conn, error) = match &l.conn {
        Conn::Connecting => ("connecting", String::new()),
        Conn::Joined => ("joined", String::new()),
        Conn::Reconnecting(e) => ("reconnecting", e.clone()),
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
<meta name="theme-color" content="#0f0f11">
<title>Bingo board</title>
<style>
:root{--bg:#0f0f11;--surface:#18181c;--raised:#222227;--border:#2c2c33;--text:#eaeaed;--muted:#8e8e96;--faint:#606068;--accent:#ceae74;--good:#7cba88;--warn:#dca858}
*{box-sizing:border-box;-webkit-tap-highlight-color:transparent}
[hidden]{display:none!important}
html,body{margin:0;height:100%;background:var(--bg);color:var(--text);font:15px/1.3 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;-webkit-font-smoothing:antialiased;overscroll-behavior:none}
body{display:flex;flex-direction:column;padding:max(14px,env(safe-area-inset-top)) 10px max(12px,env(safe-area-inset-bottom));user-select:none;-webkit-user-select:none}
header{display:flex;align-items:flex-end;justify-content:space-between;gap:12px;padding:0 2px 12px}
.eyebrow{font-size:11px;letter-spacing:.08em;text-transform:uppercase;color:var(--faint);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
#left{min-width:0}
#team{margin-top:4px;font-weight:600;font-size:17px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
#right{text-align:right;flex:none}
#clock{font-size:26px;font-weight:600;letter-spacing:-.02em;font-variant-numeric:tabular-nums;line-height:1}
#state{margin-top:6px;font-size:12px;color:var(--muted)}
#state.run{color:var(--good)}
#state.wait{color:var(--warn)}
#state.done{color:var(--accent)}
#stage{flex:1;min-height:0;display:flex;flex-direction:column;align-items:center;justify-content:center}
#scores{padding-top:14px;font-size:14px;color:var(--muted);text-align:center;line-height:1.6}
#scores span{white-space:nowrap}
#scores b{color:var(--text);font-variant-numeric:tabular-nums;margin-left:5px}
#scores .sep{margin:0 10px;color:var(--faint)}
#board{display:grid;gap:4px;width:100%;aspect-ratio:1}
.sq span{display:block;max-width:100%}
.sq{position:relative;display:flex;align-items:center;justify-content:center;text-align:center;padding:5px 2px;border-radius:9px;background:var(--surface);border:1px solid var(--border);color:#d6d6dc;font-size:var(--fs,12px);font-weight:500;line-height:1.18;overflow:hidden;transition:transform .1s ease,background-color .15s ease}
.sq:active{transform:scale(.95)}
.sq.mine{background:var(--c);border-color:transparent;color:var(--on);font-weight:650;box-shadow:inset 0 1px 0 rgba(255,255,255,.2),0 2px 10px -2px var(--c)}
.sq.taken{background:color-mix(in srgb,var(--c) 20%,var(--surface));border-color:color-mix(in srgb,var(--c) 45%,var(--border));color:var(--faint)}
.sq .bars{position:absolute;left:6px;right:6px;bottom:4px;display:flex;gap:3px}
.sq .bars i{flex:1;height:3px;border-radius:2px}
.sq.pending{animation:pulse .9s ease-in-out infinite}
@keyframes pulse{50%{opacity:.55}}
#msg{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;text-align:center;gap:14px;padding:24px;color:var(--muted);font-size:14px;line-height:1.45}
#msg b{color:var(--text);font-size:16px;font-weight:600}
#msg p{margin:0;max-width:280px}
.spin{width:22px;height:22px;border-radius:50%;border:2px solid var(--border);border-top-color:var(--accent);animation:spin .8s linear infinite}
@keyframes spin{to{transform:rotate(360deg)}}
footer{padding-top:10px;font-size:12px;color:var(--faint);text-align:center}
#awake{position:fixed;inset:0;width:100%;height:100%;opacity:0;pointer-events:none;z-index:-1}
#toast{position:fixed;left:50%;bottom:max(24px,env(safe-area-inset-bottom));transform:translate(-50%,12px);opacity:0;pointer-events:none;background:var(--raised);border:1px solid var(--border);color:var(--text);font-size:13px;padding:9px 14px;border-radius:10px;box-shadow:0 8px 24px rgba(0,0,0,.45);transition:opacity .18s,transform .18s;white-space:nowrap}
#toast.on{opacity:1;transform:translate(-50%,0)}
</style></head><body>
<header>
  <div id="left"><div class="eyebrow" id="room">Bingo</div><div id="team">Connecting</div></div>
  <div id="right"><div id="clock">0:00</div><div id="state">Connecting</div></div>
</header>
<main id="stage">
  <div id="board" hidden></div>
  <div id="msg"><div class="spin"></div><b>Connecting</b><p>Joining the lobby…</p></div>
  <div id="scores"></div>
</main>
<footer id="foot" hidden></footer>
<footer id="hint">Tap anywhere to keep the screen on</footer>
<video id="awake" src="__AWAKE__" playsinline disablepictureinpicture disableremoteplayback preload="auto" aria-hidden="true" tabindex="-1"></video>
<div id="toast"></div>
<script>
const base=location.pathname.replace(/\/$/,'');
const $=id=>document.getElementById(id);
let st=null,got=0,pending=new Map(),built='',failed=0;
const STATUS=['Waiting','Starting','Preparation','Running','Finished'];
function fmt(ms){const neg=ms<0;let s=Math.floor(Math.abs(ms)/1000);const h=Math.floor(s/3600);s%=3600;const m=Math.floor(s/60);s%=60;return(neg?'-':'')+(h?h+':'+String(m).padStart(2,'0'):m)+':'+String(s).padStart(2,'0')}
function esc(t){return String(t).replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]))}
function on(c){const m=/(\d+),\s*(\d+),\s*(\d+)/.exec(c||'');if(!m)return'#fff';const[r,g,b]=[m[1],m[2],m[3]].map(Number);return(r*299+g*587+b*114)/1000>150?'#141414':'#fff'}
function message(title,text,spin){$('board').hidden=true;$('msg').hidden=false;$('msg').innerHTML=(spin?'<div class="spin"></div>':'')+'<b>'+esc(title)+'</b>'+(text?'<p>'+esc(text)+'</p>':'')}
let toastT;function toast(t){const e=$('toast');e.textContent=t;e.classList.add('on');clearTimeout(toastT);toastT=setTimeout(()=>e.classList.remove('on'),1800)}
async function poll(){
  try{const r=await fetch(base+'/state',{cache:'no-store'});if(!r.ok)throw 0;st=await r.json();got=performance.now();failed=0;render()}
  catch(e){if(++failed>=3){message("Can't reach Bingo Hub","Check that this phone is on the same Wi-Fi as the PC and the phone board is still on.");setState('','Offline')}}
  setTimeout(poll,700)
}
function setState(cls,text){$('state').className=cls;$('state').textContent=text}
function render(){
  const s=st;
  $('room').textContent=s.room?('Lobby · '+s.room):'Bingo';
  $('team').textContent=s.myName||(s.conn==='joined'?'Spectating':'Connecting');
  $('team').style.color=s.myColor||'';
  if(s.conn==='reconnecting')setState('wait','Reconnecting');
  else if(s.paused)setState('wait','Paused');
  else setState(['','wait','wait','run','done'][s.status]||'',STATUS[s.status]||'');
  $('scores').innerHTML=s.scores.map(t=>`<span><span style="color:${t.color}">${esc(t.name)}</span><b>${t.score}</b></span>`).join('<span class="sep">·</span>');
  $('scores').hidden=!s.scores.length;
  if(s.conn==='failed')return message("Couldn't join the lobby",s.error);
  if(s.conn==='closed')return message('Phone board stopped','Start it again in Bingo Hub to keep playing.');
  if(s.conn==='connecting'||(s.conn==='reconnecting'&&!s.size))return message(s.conn==='connecting'?'Connecting':'Reconnecting','Joining the lobby…',true);
  if(!s.size)return message(s.status<2?'Waiting for the race':'Waiting for the board',s.status<2?'The board appears here when the race starts.':'',true);
  $('msg').hidden=true;
  const n=s.size,b=$('board');
  const key=n+'|'+s.squares.map(q=>q.text).join('|');
  if(key!==built){
    built=key;b.innerHTML='';pending.clear();
    b.style.gridTemplateColumns=`repeat(${n},minmax(0,1fr))`;b.style.gridTemplateRows=`repeat(${n},minmax(0,1fr))`;
    s.squares.forEach((q,i)=>{const d=document.createElement('div');d.className='sq';const t=document.createElement('span');t.textContent=q.text;const bars=document.createElement('div');bars.className='bars';d.append(t,bars);d.onclick=()=>tap(i);b.appendChild(d)});
  }
  b.hidden=false;
  const col=t=>(s.teams[t]||{}).color||'rgb(128,128,128)';
  [...b.children].forEach((d,i)=>{
    const teams=s.squares[i].teams||[];
    const mine=s.myTeam!=null&&teams.includes(s.myTeam);
    const p=pending.get(i);
    if(p!==undefined&&p!==mine)d.classList.add('pending');else{pending.delete(i);d.classList.remove('pending')}
    const others=teams.filter(t=>t!==s.myTeam);
    const taken=!mine&&s.lockout&&others.length>0;
    d.classList.toggle('mine',mine);d.classList.toggle('taken',taken);
    const fill=mine?col(s.myTeam):taken?col(others[0]):'';
    d.style.setProperty('--c',fill);d.style.setProperty('--on',on(fill));
    const strips=taken?others.slice(1):others;
    const bars=d.lastChild,html=strips.map(t=>`<i style="background:${col(t)}"></i>`).join('');
    if(bars.innerHTML!==html)bars.innerHTML=html;
  });
  fit();
  $('foot').textContent=s.notice||'';$('foot').hidden=!s.notice;
}
function fit(){
  const b=$('board'),n=st.size||5,stage=$('stage');if(b.hidden)return;
  const side=Math.max(0,Math.min(stage.clientWidth,stage.clientHeight-$('scores').offsetHeight));
  b.style.width=side+'px';
  const fs=Math.max(9,Math.min(16,side/n/6.8));
  document.documentElement.style.setProperty('--fs',fs+'px');
  // Shrink only the squares whose longest word or text still doesn't fit.
  for(const d of b.children){
    const t=d.firstChild;let f=fs;d.style.fontSize='';
    while(f>8&&(t.scrollWidth>t.clientWidth+1||t.offsetHeight>d.clientHeight-8)){f-=.5;d.style.fontSize=f+'px'}
  }
}
addEventListener('resize',()=>st&&st.size&&fit());
async function tap(i){
  const s=st;if(!s)return;
  if(s.conn!=='joined')return toast('Reconnecting to the lobby…');
  if(s.paused)return toast('The race is paused');
  if(s.status!==3)return toast(s.status===4?'The race is over':'Claims open when the race starts');
  const teams=s.squares[i].teams||[];const mine=teams.includes(s.myTeam);
  if(!mine&&s.lockout&&teams.length)return toast('Already claimed');
  pending.set(i,!mine);render();
  if(navigator.vibrate)navigator.vibrate(12);
  try{await fetch(base+'/check/'+i,{method:'POST'})}catch(e){}
}
setInterval(()=>{if(!st)return;const run=st.status>=1&&st.status<=3&&!st.paused;$('clock').textContent=st.status===0?'0:00':fmt(st.clock+(run?performance.now()-got:0));$('clock').style.color=st.status===0?'var(--faint)':''},250);
// Keep the screen on. The Wake Lock API needs https, which a page on the local network can't have,
// so fall back to a silent black clip covering the page (browsers keep the screen on for a playing
// video with sound, or one that covers the page). Not looped: iOS ignores looping clips, so rewind.
const awake=$('awake');let lock=null;
if(navigator.audioSession)navigator.audioSession.type='ambient';else awake.muted=true;
async function keepAwake(){
  if(document.visibilityState!=='visible')return;
  if(navigator.wakeLock){try{if(!lock||lock.released)lock=await navigator.wakeLock.request('screen');$('hint').hidden=true;return}catch(e){}}
  if(awake.paused)awake.play().catch(()=>{});
}
awake.addEventListener('timeupdate',()=>{if(awake.currentTime>awake.duration-1)awake.currentTime=0});
awake.addEventListener('playing',()=>$('hint').hidden=true);
awake.addEventListener('error',()=>{if(!lock)$('hint').textContent='Turn off auto-lock to keep the screen on'});
addEventListener('touchend',keepAwake);addEventListener('click',keepAwake);
document.addEventListener('visibilitychange',keepAwake);
setInterval(keepAwake,10000);keepAwake();
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

    #[test]
    fn base64_matches_rfc4648() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(i.as_bytes()), o);
        }
        assert!(page().contains("data:video/mp4;base64,AAAA") && !page().contains("__AWAKE__"));
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
        let key = random_token(10);
        let ph = Phone::start(&tag, "hubtest-admin", 1, &format!("{tag}hub"), &key);
        assert!(ph.url.ends_with(&format!("/{key}")), "the link keeps the saved key");
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
        // A dropped connection comes back by itself, and claims work again.
        ph.client.drop_connection();
        wait(&ph.client, |l| matches!(l.conn, Conn::Reconnecting(_)), "phone notices the drop");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&http(&format!("{local}/state"), "GET").1).unwrap()["conn"], "reconnecting");
        wait(&ph.client, |l| matches!(l.conn, Conn::Joined) && l.my_team() == Some(0), "phone reconnects on the admin's team");
        http(&format!("{local}/check/7"), "POST");
        wait(&admin, |l| l.squares[7].teams == vec![0], "claims work after reconnecting");
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