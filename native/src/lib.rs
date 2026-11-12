//! SOCKS5 transport. JNI is a control plane only; bytes stay in Rust.
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{oneshot, Semaphore},
    task::JoinSet,
    time::{timeout, Instant},
};

const HANDSHAKE: Duration = Duration::from_secs(15);
const IDLE: Duration = Duration::from_secs(300);
const UDP_IDLE: Duration = Duration::from_secs(60);
fn err(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub listen: String,
    pub port: u16,
    pub workers: usize,
    pub username: String,
    pub password: String,
    pub udp_listen: String,
    pub udp_port: String,
    pub bind_ipv4: String,
    pub bind_ipv6: String,
    pub interface: String,
    pub ipv6_only: bool,
    pub tcp_nodelay: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "::".into(),
            port: 1080,
            workers: 4,
            username: String::new(),
            password: String::new(),
            udp_listen: String::new(),
            udp_port: "0".into(),
            bind_ipv4: "0.0.0.0".into(),
            bind_ipv6: "::".into(),
            interface: String::new(),
            ipv6_only: false,
            tcp_nodelay: true,
        }
    }
}
impl Config {
    pub fn validate(&self) -> io::Result<()> {
        self.listen
            .parse::<IpAddr>()
            .map_err(|_| err("Listen address must be an IP address"))?;
        if self.port == 0 {
            return Err(err("Port must be between 1 and 65535"));
        }
        if !(1..=32).contains(&self.workers) {
            return Err(err("Workers must be between 1 and 32"));
        }
        if self.username.is_empty() != self.password.is_empty() {
            return Err(err("Enter both username and password"));
        }
        if self.username.len() > 255 || self.password.len() > 255 {
            return Err(err("Credentials must be at most 255 UTF-8 bytes"));
        }
        if !self.udp_listen.is_empty() {
            self.udp_listen
                .parse::<IpAddr>()
                .map_err(|_| err("Invalid UDP listen address"))?;
        }
        if !self.bind_ipv4.is_empty() {
            self.bind_ipv4
                .parse::<Ipv4Addr>()
                .map_err(|_| err("Invalid IPv4 source address"))?;
        }
        if !self.bind_ipv6.is_empty() {
            self.bind_ipv6
                .parse::<Ipv6Addr>()
                .map_err(|_| err("Invalid IPv6 source address"))?;
        }
        if self.interface.as_bytes().contains(&0) || self.interface.len() > 15 {
            return Err(err("Invalid interface name"));
        }
        port_range(&self.udp_port)?;
        Ok(())
    }
}
fn port_range(value: &str) -> io::Result<(u16, u16)> {
    let (a, b) = value.split_once('-').unwrap_or((value, value));
    let a = a
        .parse::<u16>()
        .map_err(|_| err("Invalid UDP port or range"))?;
    let b = b
        .parse::<u16>()
        .map_err(|_| err("Invalid UDP port or range"))?;
    if a > b || (a == 0 && b != 0) {
        return Err(err("Invalid UDP port range"));
    }
    Ok((a, b))
}
#[derive(Default)]
pub struct Stats {
    running: AtomicBool,
    active: AtomicU64,
    total: AtomicU64,
    uploaded: AtomicU64,
    downloaded: AtomicU64,
    failures: AtomicU64,
}
#[derive(Serialize)]
struct Snapshot {
    running: bool,
    active: u64,
    total: u64,
    uploaded: u64,
    downloaded: u64,
    failures: u64,
}
impl Stats {
    pub fn json(&self) -> String {
        serde_json::to_string(&Snapshot {
            running: self.running.load(Ordering::Relaxed),
            active: self.active.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            uploaded: self.uploaded.load(Ordering::Relaxed),
            downloaded: self.downloaded.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
        })
        .unwrap()
    }
}
struct Active(Arc<Stats>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        _ => ip,
    }
}
fn canonical_addr(a: SocketAddr) -> SocketAddr {
    SocketAddr::new(canonical(a.ip()), a.port())
}
fn socket(ip: IpAddr, udp: bool, only_v6: bool) -> io::Result<Socket> {
    let s = Socket::new(
        if ip.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        },
        if udp { Type::DGRAM } else { Type::STREAM },
        Some(if udp { Protocol::UDP } else { Protocol::TCP }),
    )?;
    if ip.is_ipv6() {
        s.set_only_v6(only_v6)?;
    }
    s.set_nonblocking(true)?;
    Ok(s)
}
fn bind_interface(s: &Socket, name: &str) -> io::Result<()> {
    if name.is_empty() {
        return Ok(());
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        s.bind_device(Some(name.as_bytes()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = s;
        Err(err("Interface binding is supported on Android/Linux only"))
    }
}
fn source(config: &Config, ipv4: bool) -> io::Result<IpAddr> {
    let value = if ipv4 {
        &config.bind_ipv4
    } else {
        &config.bind_ipv6
    };
    if value.is_empty() {
        Ok(if ipv4 {
            Ipv4Addr::UNSPECIFIED.into()
        } else {
            Ipv6Addr::UNSPECIFIED.into()
        })
    } else {
        value.parse().map_err(|_| err("Invalid source address"))
    }
}
#[derive(Clone, Debug)]
enum Address {
    Ip(SocketAddr),
    Name(String, u16),
}
impl Address {
    async fn resolve(&self) -> io::Result<Vec<SocketAddr>> {
        match self {
            Self::Ip(a) => Ok(vec![canonical_addr(*a)]),
            Self::Name(name, port) => timeout(
                Duration::from_secs(10),
                tokio::net::lookup_host((name.as_str(), *port)),
            )
            .await?
            .map(|v| v.take(32).map(canonical_addr).collect()),
        }
    }
}
async fn address<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Address> {
    let kind = r.read_u8().await?;
    let host = match kind {
        1 => {
            let mut b = [0; 4];
            r.read_exact(&mut b).await?;
            Some(IpAddr::V4(b.into()))
        }
        4 => {
            let mut b = [0; 16];
            r.read_exact(&mut b).await?;
            Some(IpAddr::V6(b.into()))
        }
        3 => None,
        _ => return Err(err("Unsupported address type")),
    };
    if let Some(ip) = host {
        return Ok(Address::Ip(SocketAddr::new(ip, r.read_u16().await?)));
    }
    let len = r.read_u8().await? as usize;
    if len == 0 {
        return Err(err("Empty domain"));
    }
    let mut b = vec![0; len];
    r.read_exact(&mut b).await?;
    let name = String::from_utf8(b).map_err(|_| err("Invalid domain"))?;
    if name.contains('\0') {
        return Err(err("Invalid domain"));
    }
    Ok(Address::Name(name, r.read_u16().await?))
}
fn encode_addr(addr: SocketAddr) -> Vec<u8> {
    let a = canonical_addr(addr);
    let mut out = Vec::with_capacity(19);
    match a.ip() {
        IpAddr::V4(ip) => {
            out.push(1);
            out.extend(ip.octets());
        }
        IpAddr::V6(ip) => {
            out.push(4);
            out.extend(ip.octets());
        }
    }
    out.extend(a.port().to_be_bytes());
    out
}
async fn reply(stream: &mut TcpStream, code: u8, addr: SocketAddr) -> io::Result<()> {
    let mut data = vec![5, code, 0];
    data.extend(encode_addr(addr));
    stream.write_all(&data).await
}
// Avoid early exit when comparing equal-length credentials.
fn equal_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |n, (a, b)| n | (a ^ b)) == 0
}
async fn negotiate(stream: &mut TcpStream, c: &Config) -> io::Result<(u8, Address)> {
    if stream.read_u8().await? != 5 {
        return Err(err("Unsupported SOCKS version"));
    }
    let count = stream.read_u8().await? as usize;
    let mut methods = vec![0; count];
    stream.read_exact(&mut methods).await?;
    let method = if c.username.is_empty() { 0 } else { 2 };
    if !methods.contains(&method) {
        stream.write_all(&[5, 255]).await?;
        return Err(err("No supported authentication method"));
    }
    stream.write_all(&[5, method]).await?;
    if method == 2 {
        let version = stream.read_u8().await?;
        let len = stream.read_u8().await? as usize;
        let mut user = vec![0; len];
        stream.read_exact(&mut user).await?;
        let len = stream.read_u8().await? as usize;
        let mut pass = vec![0; len];
        stream.read_exact(&mut pass).await?;
        let valid = version == 1
            && equal_secret(&user, c.username.as_bytes())
            && equal_secret(&pass, c.password.as_bytes());
        stream.write_all(&[1, if valid { 0 } else { 1 }]).await?;
        if !valid {
            return Err(err("Authentication failed"));
        }
    }
    let mut h = [0; 3];
    stream.read_exact(&mut h).await?;
    if h[0] != 5 || h[2] != 0 {
        return Err(err("Malformed SOCKS request"));
    }
    match address(stream).await {
        Ok(a) => Ok((h[1], a)),
        Err(e) => {
            reply(stream, 8, "0.0.0.0:0".parse().unwrap()).await?;
            Err(e)
        }
    }
}
async fn connect_one(addr: SocketAddr, c: Arc<Config>) -> io::Result<TcpStream> {
    let ip = source(&c, addr.is_ipv4())?;
    let s = socket(ip, false, false)?;
    bind_interface(&s, &c.interface)?;
    s.bind(&SocketAddr::new(ip, 0).into())?;
    let tcp = tokio::net::TcpSocket::from_std_stream(s.into());
    let stream = tcp.connect(addr).await?;
    stream.set_nodelay(c.tcp_nodelay)?;
    Ok(stream)
}
async fn connect(target: &Address, c: Arc<Config>) -> io::Result<TcpStream> {
    let addresses = target.resolve().await?;
    if addresses.is_empty() {
        return Err(err("No destination addresses"));
    }
    // Stagger address attempts instead of waiting for a broken first address.
    let mut attempts = JoinSet::new();
    for (index, addr) in addresses.into_iter().enumerate() {
        let c = c.clone();
        attempts.spawn(async move {
            tokio::time::sleep(Duration::from_millis((index as u64) * 200)).await;
            connect_one(addr, c).await
        });
    }
    let mut last = err("Connection failed");
    while let Some(result) = attempts.join_next().await {
        match result {
            Ok(Ok(s)) => return Ok(s),
            Ok(Err(e)) => last = e,
            Err(e) => last = err(e.to_string()),
        }
    }
    Err(last)
}
async fn pump<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut r: R,
    mut w: W,
    counter: &AtomicU64,
    activity: &AtomicU64,
    started: Instant,
) -> io::Result<()> {
    let mut buf = vec![0; 32768];
    loop {
        let n = match timeout(IDLE, r.read(&mut buf)).await {
            Ok(result) => result?,
            Err(e) => {
                // A long download must not expire because the upload half is quiet.
                if (started.elapsed().as_millis() as u64)
                    .saturating_sub(activity.load(Ordering::Relaxed))
                    < IDLE.as_millis() as u64
                {
                    continue;
                }
                return Err(e.into());
            }
        };
        if n == 0 {
            w.shutdown().await?;
            return Ok(());
        }
        timeout(IDLE, w.write_all(&buf[..n])).await??;
        activity.fetch_max(started.elapsed().as_millis() as u64, Ordering::Relaxed);
        counter.fetch_add(n as u64, Ordering::Relaxed);
    }
}
async fn tcp_relay(
    mut client: TcpStream,
    target: Address,
    c: Arc<Config>,
    stats: Arc<Stats>,
) -> io::Result<()> {
    let upstream = match timeout(Duration::from_secs(10), connect(&target, c)).await {
        Ok(Ok(s)) => s,
        _ => {
            reply(&mut client, 4, "0.0.0.0:0".parse().unwrap()).await?;
            return Err(err("Destination unreachable"));
        }
    };
    reply(&mut client, 0, upstream.local_addr()?).await?;
    let (cr, cw) = client.into_split();
    let (ur, uw) = upstream.into_split();
    let activity = AtomicU64::new(0);
    let started = Instant::now();
    tokio::try_join!(
        pump(cr, uw, &stats.uploaded, &activity, started),
        pump(ur, cw, &stats.downloaded, &activity, started)
    )?;
    Ok(())
}
fn udp_out(c: &Config, ipv4: bool) -> io::Result<UdpSocket> {
    let ip = source(c, ipv4)?;
    let s = socket(ip, true, !ipv4)?;
    bind_interface(&s, &c.interface)?;
    s.bind(&SocketAddr::new(ip, 0).into())?;
    UdpSocket::from_std(s.into())
}
fn udp_listener(c: &Config, local: SocketAddr) -> io::Result<UdpSocket> {
    let ip = if c.udp_listen.is_empty() {
        local.ip()
    } else {
        c.udp_listen
            .parse()
            .map_err(|_| err("Invalid UDP address"))?
    };
    let (first, last) = port_range(&c.udp_port)?;
    let mut error = err("UDP port unavailable");
    for port in first..=last {
        let s = socket(ip, true, c.ipv6_only)?;
        match s.bind(&SocketAddr::new(ip, port).into()) {
            Ok(()) => return UdpSocket::from_std(s.into()),
            Err(e) => error = e,
        }
    }
    Err(error)
}
async fn send_datagram(
    target: Address,
    payload: &[u8],
    v4: &UdpSocket,
    v6: &UdpSocket,
    allowed: &mut HashMap<SocketAddr, Instant>,
    stats: &Stats,
) -> io::Result<()> {
    let destinations = target.resolve().await?;
    allowed.retain(|_, t| t.elapsed() < UDP_IDLE);
    for dest in destinations {
        if allowed.len() >= 256 && !allowed.contains_key(&dest) {
            return Err(err("Too many UDP destinations"));
        }
        let sock = if dest.is_ipv4() { v4 } else { v6 };
        if sock.send_to(payload, dest).await.is_ok() {
            allowed.insert(dest, Instant::now());
            stats
                .uploaded
                .fetch_add(payload.len() as u64, Ordering::Relaxed);
            return Ok(());
        }
    }
    Err(err("UDP destination unreachable"))
}
fn allowed_peer(allowed: &HashMap<SocketAddr, Instant>, peer: SocketAddr) -> bool {
    allowed
        .get(&canonical_addr(peer))
        .is_some_and(|t| t.elapsed() < UDP_IDLE)
}
async fn udp_relay(
    mut control: TcpStream,
    target: Address,
    c: Arc<Config>,
    stats: Arc<Stats>,
    framed: bool,
) -> io::Result<()> {
    let local = canonical_addr(control.local_addr()?);
    let peer = canonical_addr(control.peer_addr()?);
    let sockets = (|| {
        let v4 = udp_out(&c, true)?;
        let v6 = udp_out(&c, false)?;
        let relay = if framed {
            udp_listener(
                &Config {
                    udp_port: "0".into(),
                    udp_listen: String::new(),
                    ..(*c).clone()
                },
                local,
            )?
        } else {
            udp_listener(&c, local)?
        };
        Ok::<_, io::Error>((v4, v6, relay))
    })();
    let (v4, v6, relay) = match sockets {
        Ok(s) => s,
        Err(e) => {
            reply(&mut control, 1, local).await?;
            return Err(e);
        }
    };
    reply(&mut control, 0, relay.local_addr()?).await?;
    let mut allowed = HashMap::new();
    let mut buf4 = vec![0; 65536];
    let mut buf6 = vec![0; 65536];
    let mut input = vec![0; 65536];
    let mut control_byte = [0];
    let mut endpoint = match target {
        Address::Ip(a) if a.port() != 0 => Some(SocketAddr::new(peer.ip(), a.port())),
        _ => None,
    };
    if framed {
        let (mut reader, mut writer) = control.into_split();
        // A dedicated task preserves partial frame reads when UDP responses arrive.
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mut readers = JoinSet::new();
        readers.spawn(async move {
            loop {
                let len = reader.read_u16().await? as usize;
                let header = reader.read_u8().await? as usize;
                if !(10..=255).contains(&header) {
                    return Err::<(), io::Error>(err("Invalid UDP frame header"));
                }
                let mut bytes = vec![0; header - 3];
                reader.read_exact(&mut bytes).await?;
                let mut slice = bytes.as_slice();
                let address = address(&mut slice).await?;
                if !slice.is_empty() {
                    return Err(err("Unexpected UDP header bytes"));
                }
                let mut data = vec![0; len];
                reader.read_exact(&mut data).await?;
                if tx.send((address, data)).await.is_err() {
                    return Ok(());
                }
            }
        });
        loop {
            timeout(UDP_IDLE,async {
                tokio::select! {
                    item=rx.recv()=>{let (target,data)=item.ok_or_else(||err("UDP stream closed"))?;send_datagram(target,&data,&v4,&v6,&mut allowed,&stats).await?;},
                    received=v4.recv_from(&mut buf4)=>{let (n,p)=received?;if allowed_peer(&allowed,p){write_frame(&mut writer,p,&buf4[..n],&stats).await?;}},
                    received=v6.recv_from(&mut buf6)=>{let (n,p)=received?;if allowed_peer(&allowed,p){write_frame(&mut writer,p,&buf6[..n],&stats).await?;}},
                }; Ok::<(),io::Error>(())
            }).await??;
        }
    }
    loop {
        timeout(UDP_IDLE,async {
            tokio::select! {
                result=control.read(&mut control_byte)=>{if result?==0{return Err(err("UDP control closed"));}},
                received=relay.recv_from(&mut input)=>{
                    let (n,from)=received?;let from=canonical_addr(from);
                    if from.ip()==peer.ip() && endpoint.is_none_or(|e|e==from) && n>=4 && input[..3]==[0,0,0] {
                        let mut bytes=&input[3..n];
                        if let Ok(dest)=address(&mut bytes).await {
                            endpoint=Some(from);
                            // Bad destination datagrams do not tear down an association.
                            let _=send_datagram(dest,bytes,&v4,&v6,&mut allowed,&stats).await;
                        }
                    }
                },
                received=v4.recv_from(&mut buf4)=>{let (n,p)=received?;if let Some(e)=endpoint{if allowed_peer(&allowed,p){write_udp(&relay,e,p,&buf4[..n],&stats).await?;}}},
                received=v6.recv_from(&mut buf6)=>{let (n,p)=received?;if let Some(e)=endpoint{if allowed_peer(&allowed,p){write_udp(&relay,e,p,&buf6[..n],&stats).await?;}}},
            }; Ok::<(),io::Error>(())
        }).await??;
    }
}
async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    peer: SocketAddr,
    payload: &[u8],
    stats: &Stats,
) -> io::Result<()> {
    let address = encode_addr(peer);
    let mut data = Vec::with_capacity(payload.len() + 22);
    data.extend((payload.len() as u16).to_be_bytes());
    data.push((address.len() + 3) as u8);
    data.extend(address);
    data.extend(payload);
    timeout(UDP_IDLE, w.write_all(&data)).await??;
    stats
        .downloaded
        .fetch_add(payload.len() as u64, Ordering::Relaxed);
    Ok(())
}
async fn write_udp(
    s: &UdpSocket,
    client: SocketAddr,
    peer: SocketAddr,
    payload: &[u8],
    stats: &Stats,
) -> io::Result<()> {
    let mut data = vec![0, 0, 0];
    data.extend(encode_addr(peer));
    data.extend(payload);
    s.send_to(&data, client).await?;
    stats
        .downloaded
        .fetch_add(payload.len() as u64, Ordering::Relaxed);
    Ok(())
}
async fn session(mut stream: TcpStream, c: Arc<Config>, stats: Arc<Stats>) -> io::Result<()> {
    stream.set_nodelay(c.tcp_nodelay)?;
    let (cmd, target) = timeout(HANDSHAKE, negotiate(&mut stream, &c)).await??;
    match cmd {
        1 => tcp_relay(stream, target, c, stats).await,
        3 | 5 => udp_relay(stream, target, c, stats, cmd == 5).await,
        _ => {
            reply(&mut stream, 7, "0.0.0.0:0".parse().unwrap()).await?;
            Err(err("Unsupported command"))
        }
    }
}
pub async fn serve(
    c: Config,
    stats: Arc<Stats>,
    mut stop: oneshot::Receiver<()>,
    ready: std::sync::mpsc::SyncSender<Result<(), String>>,
) -> io::Result<()> {
    let listener = (|| {
        c.validate()?;
        let ip = c.listen.parse::<IpAddr>().map_err(|e| err(e.to_string()))?;
        let s = socket(ip, false, c.ipv6_only)?;
        s.set_reuse_address(true)?;
        s.bind(&SocketAddr::new(ip, c.port).into())?;
        s.listen(256)?;
        TcpListener::from_std(s.into())
    })();
    let listener = match listener {
        Ok(l) => l,
        Err(e) => {
            let _ = ready.send(Err(e.to_string()));
            return Err(e);
        }
    };
    let c = Arc::new(c);
    let permits = Arc::new(Semaphore::new(512));
    let mut tasks = JoinSet::new();
    stats.running.store(true, Ordering::Relaxed);
    let _ = ready.send(Ok(()));
    loop {
        tokio::select! {
            _=&mut stop=>break,
            Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
            result=listener.accept()=>{
                let (stream,_)=match result{Ok(v)=>v,Err(_)=>{tokio::time::sleep(Duration::from_millis(100)).await;continue;}};
                let permit=match permits.clone().try_acquire_owned(){Ok(p)=>p,Err(_)=>{drop(stream);continue;}};
                let c=c.clone();let stats=stats.clone();
                stats.active.fetch_add(1,Ordering::Relaxed);stats.total.fetch_add(1,Ordering::Relaxed);
                let active=Active(stats.clone());
                tasks.spawn(async move{let _permit=permit;let _active=active;if session(stream,c,stats.clone()).await.is_err(){stats.failures.fetch_add(1,Ordering::Relaxed);}});
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    stats.running.store(false, Ordering::Relaxed);
    Ok(())
}
struct Engine {
    stop: oneshot::Sender<()>,
    thread: thread::JoinHandle<()>,
    stats: Arc<Stats>,
}
static ENGINE: OnceLock<Mutex<Option<Engine>>> = OnceLock::new();
fn engine() -> &'static Mutex<Option<Engine>> {
    ENGINE.get_or_init(|| Mutex::new(None))
}
pub fn start(json: &str) -> Result<(), String> {
    let c: Config = serde_json::from_str(json).map_err(|e| e.to_string())?;
    c.validate().map_err(|e| e.to_string())?;
    let mut guard = engine().lock().map_err(|_| "Engine lock failed")?;
    if guard.is_some() {
        return Err("Proxy is already running".into());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(c.workers)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let stats = Arc::new(Stats::default());
    let shared = stats.clone();
    let (tx, rx) = oneshot::channel();
    let (ready, wait) = std::sync::mpsc::sync_channel(1);
    let thread = thread::Builder::new()
        .name("socks5-rust".into())
        .spawn(move || {
            let _ = runtime.block_on(serve(c, shared.clone(), rx, ready));
            shared.running.store(false, Ordering::Relaxed);
        })
        .map_err(|e| e.to_string())?;
    match wait.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {
            *guard = Some(Engine {
                stop: tx,
                thread,
                stats,
            });
            Ok(())
        }
        result => {
            let _ = tx.send(());
            let _ = thread.join();
            Err(match result {
                Ok(Err(e)) => e,
                _ => "Proxy startup timed out".into(),
            })
        }
    }
}
pub fn stop() {
    if let Ok(mut guard) = engine().lock() {
        if let Some(e) = guard.take() {
            let _ = e.stop.send(());
            let _ = e.thread.join();
        }
    }
}
pub fn snapshot() -> String {
    engine()
        .lock()
        .ok()
        .and_then(|e| e.as_ref().map(|e| e.stats.json()))
        .unwrap_or_else(|| Stats::default().json())
}
#[cfg(target_os = "android")]
mod android {
    use super::*;
    use jni::{
        objects::{JObject, JString},
        sys::jstring,
        JNIEnv,
    };
    #[no_mangle]
    pub extern "system" fn Java_de_shyim_sockspring_NativeBridge_start(
        mut env: JNIEnv,
        _obj: JObject,
        json: JString,
    ) -> jstring {
        let result = env
            .get_string(&json)
            .map(|s| String::from(s))
            .map_err(|e| e.to_string())
            .and_then(|s| start(&s));
        env.new_string(result.err().unwrap_or_default())
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
    #[no_mangle]
    pub extern "system" fn Java_de_shyim_sockspring_NativeBridge_stop(_env: JNIEnv, _obj: JObject) {
        stop();
    }
    #[no_mangle]
    pub extern "system" fn Java_de_shyim_sockspring_NativeBridge_snapshot(
        env: JNIEnv,
        _obj: JObject,
    ) -> jstring {
        env.new_string(snapshot())
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
}

#[cfg(test)]
mod idle_tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn one_way_traffic_keeps_both_halves_alive() {
        let (mut client, client_end) = tokio::io::duplex(1024);
        let (target_end, mut target) = tokio::io::duplex(1024);
        let task = tokio::spawn(async move {
            let (cr, cw) = tokio::io::split(client_end);
            let (tr, tw) = tokio::io::split(target_end);
            let counter = AtomicU64::new(0);
            let activity = AtomicU64::new(0);
            let started = Instant::now();
            tokio::try_join!(
                pump(cr, tw, &counter, &activity, started),
                pump(tr, cw, &counter, &activity, started)
            )
        });
        for _ in 0..7 {
            target.write_all(b"x").await.unwrap();
            assert_eq!(client.read_u8().await.unwrap(), b'x');
            tokio::time::advance(Duration::from_secs(100)).await;
            tokio::task::yield_now().await;
            assert!(!task.is_finished());
        }
        client.shutdown().await.unwrap();
        target.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }
}
