//! Userland WireGuard server. Decrypted client packets never reach a TUN device:
//! TCP is terminated by an in-process stack and re-originated as ordinary sockets,
//! UDP and ICMP echo are translated per flow.
use super::{canonical, canonical_addr, connect_one, err, udp_out, Config, Stats, IDLE};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use boringtun::{
    noise::{handshake::parse_handshake_anon, rate_limiter::RateLimiter, Packet, Tunn, TunnResult},
    x25519::{PublicKey, StaticSecret},
};
use serde::{Deserialize, Serialize};
use smoltcp::{
    iface::{Interface, PollResult, SocketHandle, SocketSet},
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    socket::tcp,
    wire::{HardwareAddress, IpCidr, IpListenEndpoint},
};
use std::{
    collections::{HashMap, VecDeque},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    ops::Range,
    sync::{atomic::Ordering, Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    sync::{mpsc, Notify},
    task::{AbortHandle, JoinSet},
    time::{timeout, Instant},
};

const MAX_PEERS: usize = 64;
const MAX_TCP: usize = 512;
const MAX_UDP: usize = 1024;
const MAX_PING: usize = 64;
const TCP_BUFFER: usize = 256 * 1024;
const UDP_FLOW_IDLE: Duration = Duration::from_secs(120);
const PING_IDLE: Duration = Duration::from_secs(30);
const CONNECT: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(250);
// Handshakes per second before boringtun demands cookies.
const HANDSHAKE_LIMIT: u64 = 100;
const WG_OVERHEAD: usize = 32;

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct WgConfig {
    pub enabled: bool,
    pub listen: String,
    pub port: u16,
    pub private_key: String,
    pub address4: String,
    pub address6: String,
    pub mtu: u16,
    /// Resolvers that answer queries sent to the tunnel address. `ip` or `ip:port`.
    pub dns: Vec<String>,
    pub peers: Vec<WgPeer>,
    /// Loopback destinations are refused unless a test asks for them.
    pub allow_loopback: bool,
}
impl Default for WgConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "::".into(),
            port: 51820,
            private_key: String::new(),
            address4: "10.77.0.1".into(),
            address6: "fd77:736f:636b::1".into(),
            mtu: 1420,
            dns: Vec::new(),
            peers: Vec::new(),
            allow_loopback: false,
        }
    }
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct WgPeer {
    pub public_key: String,
    pub preshared_key: String,
    pub allowed_ips: Vec<String>,
}
#[derive(Clone, Serialize)]
pub struct PeerStatus {
    pub public_key: String,
    pub endpoint: String,
    /// Seconds since the last completed handshake, -1 if there was none.
    pub handshake_age: i64,
    pub received: u64,
    pub sent: u64,
}

fn key(value: &str, what: &str) -> io::Result<[u8; 32]> {
    STANDARD
        .decode(value.trim())
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
        .ok_or_else(|| err(format!("Invalid WireGuard {what}")))
}
fn cidr(value: &str) -> io::Result<(IpAddr, u8)> {
    let (ip, len) = value.split_once('/').unwrap_or((value, ""));
    let ip: IpAddr = ip
        .trim()
        .parse()
        .map_err(|_| err("Invalid WireGuard allowed IP"))?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    let len = if len.is_empty() {
        max
    } else {
        len.trim()
            .parse::<u8>()
            .ok()
            .filter(|n| *n <= max)
            .ok_or_else(|| err("Invalid WireGuard allowed IP prefix"))?
    };
    Ok((ip, len))
}
fn contains(net: (IpAddr, u8), ip: IpAddr) -> bool {
    fn bits(a: &[u8], b: &[u8], len: u8) -> bool {
        let full = (len / 8) as usize;
        let rest = len % 8;
        a[..full] == b[..full] && (rest == 0 || (a[full] ^ b[full]) >> (8 - rest) == 0)
    }
    match (net.0, ip) {
        (IpAddr::V4(a), IpAddr::V4(b)) => bits(&a.octets(), &b.octets(), net.1),
        (IpAddr::V6(a), IpAddr::V6(b)) => bits(&a.octets(), &b.octets(), net.1),
        _ => false,
    }
}
fn resolver(value: &str) -> Option<SocketAddr> {
    let value = value.trim();
    value
        .parse::<SocketAddr>()
        .ok()
        .or_else(|| value.parse::<IpAddr>().ok().map(|ip| (ip, 53).into()))
        .map(canonical_addr)
}

impl WgConfig {
    pub fn validate(&self) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        self.listen
            .parse::<IpAddr>()
            .map_err(|_| err("WireGuard listen address must be an IP address"))?;
        if self.port == 0 {
            return Err(err("WireGuard port must be between 1 and 65535"));
        }
        key(&self.private_key, "private key")?;
        self.address4
            .parse::<Ipv4Addr>()
            .map_err(|_| err("Invalid WireGuard IPv4 tunnel address"))?;
        self.address6
            .parse::<Ipv6Addr>()
            .map_err(|_| err("Invalid WireGuard IPv6 tunnel address"))?;
        if !(1280..=1500).contains(&self.mtu) {
            return Err(err("WireGuard MTU must be between 1280 and 1500"));
        }
        if self.peers.len() > MAX_PEERS {
            return Err(err("Too many WireGuard devices"));
        }
        let mut seen = Vec::new();
        for peer in &self.peers {
            let public = key(&peer.public_key, "public key")?;
            if seen.contains(&public) {
                return Err(err("Duplicate WireGuard public key"));
            }
            seen.push(public);
            if !peer.preshared_key.is_empty() {
                key(&peer.preshared_key, "preshared key")?;
            }
            for ip in &peer.allowed_ips {
                cidr(ip)?;
            }
        }
        for dns in &self.dns {
            resolver(dns).ok_or_else(|| err("Invalid WireGuard DNS server"))?;
        }
        Ok(())
    }
}

/// Derives the base64 public key for a base64 private key.
pub fn public_key(private: &str) -> Result<String, String> {
    let secret = StaticSecret::from(key(private, "private key").map_err(|e| e.to_string())?);
    Ok(STANDARD.encode(PublicKey::from(&secret).as_bytes()))
}

type Resolvers = Arc<RwLock<Vec<SocketAddr>>>;
// The running server's resolver list, so Android can follow network changes.
static RESOLVERS: Mutex<Option<Resolvers>> = Mutex::new(None);
/// Replaces the resolvers behind the tunnel address of the running server. The
/// list is comma or whitespace separated; an empty list keeps the current one.
pub fn set_dns(list: &str) {
    let parsed: Vec<SocketAddr> = list
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(resolver)
        .collect();
    if parsed.is_empty() {
        return;
    }
    if let Some(current) = RESOLVERS.lock().ok().and_then(|g| g.clone()) {
        if let Ok(mut guard) = current.write() {
            *guard = parsed;
        }
    }
}
fn system_resolvers() -> Vec<SocketAddr> {
    std::fs::read_to_string("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        .filter_map(resolver)
        .collect()
}

// ---- packet helpers ------------------------------------------------------

const TCP: u8 = 6;
const UDP: u8 = 17;
const ICMP: u8 = 1;
const ICMP6: u8 = 58;

struct Ip<'a> {
    src: IpAddr,
    dst: IpAddr,
    proto: u8,
    /// Where the transport header starts.
    offset: usize,
    payload: &'a [u8],
}
fn parse_ip(p: &[u8]) -> Option<Ip<'_>> {
    match p.first()? >> 4 {
        4 => {
            let header = ((p[0] & 15) as usize) * 4;
            let total = u16::from_be_bytes([*p.get(2)?, *p.get(3)?]) as usize;
            if header < 20 || total < header || p.len() < total {
                return None;
            }
            // Fragments carry no transport header to translate.
            if u16::from_be_bytes([p[6], p[7]]) & 0x3fff != 0 {
                return None;
            }
            Some(Ip {
                src: Ipv4Addr::new(p[12], p[13], p[14], p[15]).into(),
                dst: Ipv4Addr::new(p[16], p[17], p[18], p[19]).into(),
                proto: p[9],
                offset: header,
                payload: &p[header..total],
            })
        }
        6 => {
            if p.len() < 40 {
                return None;
            }
            let len = u16::from_be_bytes([p[4], p[5]]) as usize;
            let payload = p.get(40..40 + len)?;
            let src: [u8; 16] = p[8..24].try_into().ok()?;
            let dst: [u8; 16] = p[24..40].try_into().ok()?;
            Some(Ip {
                src: Ipv6Addr::from(src).into(),
                dst: Ipv6Addr::from(dst).into(),
                proto: p[6],
                offset: 40,
                payload,
            })
        }
        _ => None,
    }
}
fn sum(data: &[u8], mut acc: u32) -> u32 {
    let mut chunks = data.chunks_exact(2);
    for c in &mut chunks {
        acc += u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    if let [last] = chunks.remainder() {
        acc += (*last as u32) << 8;
    }
    acc
}
fn fold(mut acc: u32) -> u16 {
    while acc >> 16 != 0 {
        acc = (acc & 0xffff) + (acc >> 16);
    }
    !(acc as u16)
}
fn pseudo(src: IpAddr, dst: IpAddr, proto: u8, len: usize) -> u32 {
    let mut acc = proto as u32 + len as u32;
    match (src, dst) {
        (IpAddr::V4(s), IpAddr::V4(d)) => {
            acc = sum(&s.octets(), acc);
            acc = sum(&d.octets(), acc);
        }
        (IpAddr::V6(s), IpAddr::V6(d)) => {
            acc = sum(&s.octets(), acc);
            acc = sum(&d.octets(), acc);
        }
        _ => {}
    }
    acc
}
/// Builds an IP packet around `payload`. Both addresses share one family.
fn build_ip(src: IpAddr, dst: IpAddr, proto: u8, payload: &[u8]) -> Option<Vec<u8>> {
    match (src, dst) {
        (IpAddr::V4(s), IpAddr::V4(d)) => {
            let total = u16::try_from(20 + payload.len()).ok()?;
            let mut p = Vec::with_capacity(total as usize);
            p.extend([0x45, 0]);
            p.extend(total.to_be_bytes());
            p.extend([0, 0, 0x40, 0, 64, proto, 0, 0]);
            p.extend(s.octets());
            p.extend(d.octets());
            let check = fold(sum(&p, 0));
            p[10..12].copy_from_slice(&check.to_be_bytes());
            p.extend(payload);
            Some(p)
        }
        (IpAddr::V6(s), IpAddr::V6(d)) => {
            let len = u16::try_from(payload.len()).ok()?;
            let mut p = Vec::with_capacity(40 + payload.len());
            p.extend([0x60, 0, 0, 0]);
            p.extend(len.to_be_bytes());
            p.extend([proto, 64]);
            p.extend(s.octets());
            p.extend(d.octets());
            p.extend(payload);
            Some(p)
        }
        _ => None,
    }
}
fn build_udp(src: SocketAddr, dst: SocketAddr, payload: &[u8]) -> Option<Vec<u8>> {
    let len = u16::try_from(8 + payload.len()).ok()?;
    let mut udp = Vec::with_capacity(len as usize);
    udp.extend(src.port().to_be_bytes());
    udp.extend(dst.port().to_be_bytes());
    udp.extend(len.to_be_bytes());
    udp.extend([0, 0]);
    udp.extend(payload);
    let check = match fold(sum(&udp, pseudo(src.ip(), dst.ip(), UDP, udp.len()))) {
        0 => 0xffff,
        n => n,
    };
    udp[6..8].copy_from_slice(&check.to_be_bytes());
    build_ip(src.ip(), dst.ip(), UDP, &udp)
}
/// Builds an echo reply from `src` to `dst`. `message` is an ICMP message whose
/// identifier and checksum are rewritten.
fn build_echo_reply(src: IpAddr, dst: IpAddr, id: [u8; 2], message: &[u8]) -> Option<Vec<u8>> {
    if message.len() < 8 {
        return None;
    }
    let mut icmp = message.to_vec();
    let v4 = src.is_ipv4();
    icmp[0] = if v4 { 0 } else { 129 };
    icmp[1] = 0;
    icmp[2..4].fill(0);
    icmp[4..6].copy_from_slice(&id);
    let acc = if v4 {
        0
    } else {
        pseudo(src, dst, ICMP6, icmp.len())
    };
    let check = fold(sum(&icmp, acc));
    icmp[2..4].copy_from_slice(&check.to_be_bytes());
    build_ip(src, dst, if v4 { ICMP } else { ICMP6 }, &icmp)
}

// ---- TCP stack glue ------------------------------------------------------

#[derive(Default)]
struct Queue {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
    mtu: usize,
}
struct Rx(Vec<u8>);
struct Tx<'a>(&'a mut VecDeque<Vec<u8>>);
impl RxToken for Rx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}
impl TxToken for Tx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut packet = vec![0; len];
        let result = f(&mut packet);
        self.0.push_back(packet);
        result
    }
}
impl Device for Queue {
    type RxToken<'a> = Rx;
    type TxToken<'a> = Tx<'a>;
    fn receive(
        &mut self,
        _: smoltcp::time::Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let packet = self.rx.pop_front()?;
        Some((Rx(packet), Tx(&mut self.tx)))
    }
    fn transmit(&mut self, _: smoltcp::time::Instant) -> Option<Self::TxToken<'_>> {
        Some(Tx(&mut self.tx))
    }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = self.mtu;
        caps
    }
}

enum Down {
    Data(Vec<u8>),
    Eof,
    Reset,
}
type FlowKey = (SocketAddr, SocketAddr);
enum Flow {
    /// The client's SYN waits here until the real destination answers.
    Connecting(Vec<u8>),
    Open(Box<OpenFlow>),
}
struct OpenFlow {
    handle: SocketHandle,
    up: Option<mpsc::Sender<Vec<u8>>>,
    down: mpsc::Receiver<Down>,
    pending: Option<(Vec<u8>, usize)>,
    upstream_done: bool,
    closing: bool,
    last: Instant,
    task: AbortHandle,
}

/// Moves bytes between the real destination and the channels the stack drains.
async fn relay(
    stream: TcpStream,
    mut up: mpsc::Receiver<Vec<u8>>,
    down: mpsc::Sender<Down>,
    wake: Arc<Notify>,
) {
    let (mut reader, mut writer) = stream.into_split();
    let upload = async {
        while let Some(data) = up.recv().await {
            // Capacity freed: the stack may read more from the client.
            wake.notify_one();
            timeout(IDLE, writer.write_all(&data)).await??;
        }
        writer.shutdown().await
    };
    let download = async {
        let mut buf = vec![0; 32768];
        loop {
            let n = reader.read(&mut buf).await?;
            let item = if n == 0 {
                Down::Eof
            } else {
                Down::Data(buf[..n].to_vec())
            };
            if down.send(item).await.is_err() {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            wake.notify_one();
            if n == 0 {
                return Ok::<(), io::Error>(());
            }
        }
    };
    if tokio::try_join!(upload, download).is_err() {
        let _ = down.send(Down::Reset).await;
        wake.notify_one();
    }
}

// ---- UDP and ICMP translation -------------------------------------------

struct Datagram {
    /// Where the payload really goes.
    to: SocketAddr,
    /// The address the client believes it is talking to.
    shown: SocketAddr,
    data: Vec<u8>,
}
async fn udp_flow(
    socket: UdpSocket,
    client: SocketAddr,
    mut input: mpsc::Receiver<Datagram>,
    output: mpsc::Sender<Vec<u8>>,
    stats: Arc<Stats>,
    mtu: usize,
) {
    let mut contacted: HashMap<SocketAddr, (SocketAddr, Instant)> = HashMap::new();
    let mut buf = vec![0; 65536];
    let mut last = Instant::now();
    loop {
        tokio::select! {
            item = input.recv() => {
                let Some(item) = item else { return };
                contacted.retain(|_, (_, t)| t.elapsed() < UDP_FLOW_IDLE);
                if contacted.len() >= 256 && !contacted.contains_key(&item.to) {
                    continue;
                }
                if socket.send_to(&item.data, item.to).await.is_ok() {
                    contacted.insert(item.to, (item.shown, Instant::now()));
                    stats.uploaded.fetch_add(item.data.len() as u64, Ordering::Relaxed);
                    last = Instant::now();
                }
            }
            received = socket.recv_from(&mut buf) => {
                let Ok((n, from)) = received else { continue };
                // Replies are accepted only from destinations this client contacted.
                let Some((shown, _)) = contacted.get(&canonical_addr(from)) else { continue };
                let Some(packet) = build_udp(*shown, client, &buf[..n]) else { continue };
                if packet.len() > mtu {
                    continue;
                }
                stats.downloaded.fetch_add(n as u64, Ordering::Relaxed);
                last = Instant::now();
                if output.send(packet).await.is_err() {
                    return;
                }
            }
            _ = tokio::time::sleep_until(last + UDP_FLOW_IDLE) => return,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn ping_socket(c: &Config, target: IpAddr) -> io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    // Unprivileged ICMP datagram socket: the kernel owns the echo identifier.
    let (domain, protocol) = if target.is_ipv4() {
        (Domain::IPV4, Protocol::ICMPV4)
    } else {
        (Domain::IPV6, Protocol::ICMPV6)
    };
    let s = Socket::new(domain, Type::DGRAM, Some(protocol))?;
    s.set_nonblocking(true)?;
    super::bind_interface(&s, &c.interface)?;
    let ip = super::source(c, target.is_ipv4())?;
    if !ip.is_unspecified() {
        s.bind(&SocketAddr::new(ip, 0).into())?;
    }
    s.connect(&SocketAddr::new(target, 0).into())?;
    let fd: std::os::fd::OwnedFd = s.into();
    UdpSocket::from_std(fd.into())
}
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn ping_socket(_: &Config, _: IpAddr) -> io::Result<UdpSocket> {
    Err(err("ICMP relay is supported on Android/Linux only"))
}
async fn ping_flow(
    socket: UdpSocket,
    client: IpAddr,
    target: IpAddr,
    id: [u8; 2],
    mut input: mpsc::Receiver<Vec<u8>>,
    output: mpsc::Sender<Vec<u8>>,
) {
    let mut buf = vec![0; 2048];
    let mut last = Instant::now();
    loop {
        tokio::select! {
            item = input.recv() => {
                let Some(message) = item else { return };
                if socket.send(&message).await.is_ok() {
                    last = Instant::now();
                }
            }
            received = socket.recv(&mut buf) => {
                let Ok(n) = received else { continue };
                let expected = if target.is_ipv4() { 0 } else { 129 };
                if n < 8 || buf[0] != expected {
                    continue;
                }
                let Some(packet) = build_echo_reply(target, client, id, &buf[..n]) else { continue };
                if output.send(packet).await.is_err() {
                    return;
                }
            }
            _ = tokio::time::sleep_until(last + PING_IDLE) => return,
        }
    }
}

// ---- server --------------------------------------------------------------

struct Peer {
    tunn: Tunn,
    public: [u8; 32],
    allowed: Vec<(IpAddr, u8)>,
    endpoint: Option<SocketAddr>,
}
enum Event {
    Connected(FlowKey, io::Result<TcpStream>),
}

pub struct Server {
    socket: UdpSocket,
    secret: StaticSecret,
    public: PublicKey,
    limiter: Arc<RateLimiter>,
    peers: Vec<Peer>,
    config: Arc<Config>,
    stats: Arc<Stats>,
    resolvers: Resolvers,
    gateway4: Ipv4Addr,
    gateway6: Ipv6Addr,
    mtu: usize,
    allow_loopback: bool,
    started: Instant,
    iface: Interface,
    queue: Queue,
    sockets: SocketSet<'static>,
    flows: HashMap<FlowKey, Flow>,
    udp: HashMap<(SocketAddr, bool), mpsc::Sender<Datagram>>,
    pings: HashMap<(IpAddr, IpAddr, [u8; 2]), mpsc::Sender<Vec<u8>>>,
    tasks: JoinSet<()>,
    events: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
    outbound: (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>),
    wake: Arc<Notify>,
    scratch: Vec<u8>,
}

impl Server {
    /// Binds the UDP port and prepares all state. Nothing runs until `run`.
    pub fn bind(config: Arc<Config>, stats: Arc<Stats>) -> io::Result<Self> {
        let wg = &config.wireguard;
        wg.validate()?;
        let secret = StaticSecret::from(key(&wg.private_key, "private key")?);
        let public = PublicKey::from(&secret);
        let limiter = Arc::new(RateLimiter::new(&public, HANDSHAKE_LIMIT));
        let mut peers = Vec::new();
        for (index, peer) in wg.peers.iter().enumerate() {
            let peer_public = key(&peer.public_key, "public key")?;
            let preshared = if peer.preshared_key.is_empty() {
                None
            } else {
                Some(key(&peer.preshared_key, "preshared key")?)
            };
            peers.push(Peer {
                tunn: Tunn::new(
                    secret.clone(),
                    PublicKey::from(peer_public),
                    preshared,
                    None,
                    index as u32,
                    Some(limiter.clone()),
                ),
                public: peer_public,
                allowed: peer
                    .allowed_ips
                    .iter()
                    .map(|v| cidr(v))
                    .collect::<io::Result<_>>()?,
                endpoint: None,
            });
        }
        let ip: IpAddr = wg.listen.parse().map_err(|_| err("Invalid address"))?;
        let s = super::socket(ip, true, false)?;
        s.bind(&SocketAddr::new(ip, wg.port).into())?;
        let socket = UdpSocket::from_std(s.into())?;

        let gateway4: Ipv4Addr = wg.address4.parse().map_err(|_| err("Invalid address"))?;
        let gateway6: Ipv6Addr = wg.address6.parse().map_err(|_| err("Invalid address"))?;
        let mtu = wg.mtu as usize;
        let mut queue = Queue {
            mtu,
            ..Queue::default()
        };
        let mut iface_config = smoltcp::iface::Config::new(HardwareAddress::Ip);
        iface_config.random_seed = u64::from_le_bytes(
            StaticSecret::random_from_rng(rand_core::OsRng).to_bytes()[..8]
                .try_into()
                .unwrap(),
        );
        let mut iface = Interface::new(iface_config, &mut queue, smoltcp::time::Instant::ZERO);
        // The stack answers for every destination a client dials, not only its own address.
        iface.set_any_ip(true);
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::new(gateway4.into(), 32));
            let _ = addrs.push(IpCidr::new(gateway6.into(), 128));
        });
        let _ = iface.routes_mut().add_default_ipv4_route(gateway4);
        let _ = iface.routes_mut().add_default_ipv6_route(gateway6);

        let mut resolvers: Vec<SocketAddr> = wg.dns.iter().filter_map(|v| resolver(v)).collect();
        if resolvers.is_empty() {
            resolvers = system_resolvers();
        }
        let resolvers = Arc::new(RwLock::new(resolvers));
        if let Ok(mut guard) = RESOLVERS.lock() {
            *guard = Some(resolvers.clone());
        }
        Ok(Self {
            socket,
            secret,
            public,
            limiter,
            peers,
            stats,
            resolvers,
            gateway4,
            gateway6,
            mtu,
            allow_loopback: wg.allow_loopback,
            started: Instant::now(),
            iface,
            queue,
            sockets: SocketSet::new(Vec::new()),
            flows: HashMap::new(),
            udp: HashMap::new(),
            pings: HashMap::new(),
            tasks: JoinSet::new(),
            events: mpsc::channel(64),
            outbound: mpsc::channel(256),
            wake: Arc::new(Notify::new()),
            scratch: vec![0; 65536 + WG_OVERHEAD],
            config,
        })
    }

    /// Serves until the future is dropped. Dropping it cancels every flow.
    pub async fn run(mut self) -> io::Result<()> {
        let mut datagram = vec![0; 65536];
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut ticks = 0u32;
        loop {
            let delay = self
                .iface
                .poll_delay(self.now(), &self.sockets)
                .map(|d| Duration::from_micros(d.total_micros()));
            let mut pump = false;
            tokio::select! {
                received = self.socket.recv_from(&mut datagram) => {
                    match received {
                        Ok((n, from)) => self.datagram(&datagram[..n], from).await,
                        // ICMP errors for departed peers surface here on some kernels.
                        Err(_) => tokio::time::sleep(Duration::from_millis(1)).await,
                    }
                }
                Some(packet) = self.outbound.1.recv() => self.send_ip(&packet).await,
                Some(event) = self.events.1.recv() => self.event(event),
                _ = self.wake.notified() => pump = true,
                Some(_) = self.tasks.join_next(), if !self.tasks.is_empty() => {}
                _ = tokio::time::sleep(delay.unwrap_or(TICK)), if delay.is_some() => {}
                _ = tick.tick() => {
                    ticks = ticks.wrapping_add(1);
                    self.timers(ticks.is_multiple_of(4)).await;
                    pump = ticks.is_multiple_of(4);
                }
            }
            self.poll(pump).await;
        }
    }

    fn now(&self) -> smoltcp::time::Instant {
        smoltcp::time::Instant::from_micros(self.started.elapsed().as_micros() as i64)
    }

    async fn send_to_peer(&self, index: usize, len: usize) {
        if let Some(endpoint) = self.peers[index].endpoint {
            let _ = self.socket.send_to(&self.scratch[..len], endpoint).await;
        }
    }

    async fn datagram(&mut self, data: &[u8], from: SocketAddr) {
        let ip = canonical(from.ip());
        let mut scratch = std::mem::take(&mut self.scratch);
        let parsed = match self.limiter.verify_packet(Some(ip), data, &mut scratch) {
            Ok(parsed) => parsed,
            Err(TunnResult::WriteToNetwork(cookie)) => {
                let _ = self.socket.send_to(cookie, from).await;
                self.scratch = scratch;
                return;
            }
            Err(_) => {
                self.scratch = scratch;
                return;
            }
        };
        let index = match parsed {
            Packet::HandshakeInit(p) => parse_handshake_anon(&self.secret, &self.public, &p)
                .ok()
                .and_then(|h| {
                    self.peers
                        .iter()
                        .position(|peer| peer.public == h.peer_static_public)
                }),
            Packet::HandshakeResponse(p) => Some((p.receiver_idx >> 8) as usize),
            Packet::PacketCookieReply(p) => Some((p.receiver_idx >> 8) as usize),
            Packet::PacketData(p) => Some((p.receiver_idx >> 8) as usize),
        };
        let Some(index) = index.filter(|i| *i < self.peers.len()) else {
            self.scratch = scratch;
            return;
        };
        let mut inner = None;
        let mut replies = Vec::new();
        let mut authentic = true;
        match self.peers[index]
            .tunn
            .decapsulate(Some(ip), data, &mut scratch)
        {
            TunnResult::Done => {}
            TunnResult::Err(_) => authentic = false,
            TunnResult::WriteToNetwork(reply) => {
                replies.push(reply.to_vec());
                // A completed handshake releases packets queued for this peer.
                while let TunnResult::WriteToNetwork(more) =
                    self.peers[index].tunn.decapsulate(None, &[], &mut scratch)
                {
                    replies.push(more.to_vec());
                }
            }
            TunnResult::WriteToTunnelV4(packet, src) => inner = Some((packet.to_vec(), src.into())),
            TunnResult::WriteToTunnelV6(packet, src) => inner = Some((packet.to_vec(), src.into())),
        }
        self.scratch = scratch;
        if !authentic {
            return;
        }
        // Roaming: answer wherever the last authenticated packet came from.
        self.peers[index].endpoint = Some(from);
        for reply in replies {
            let _ = self.socket.send_to(&reply, from).await;
        }
        if let Some((packet, src)) = inner {
            let src: IpAddr = src;
            if self.peers[index].allowed.iter().any(|n| contains(*n, src)) {
                self.inbound(index, packet).await;
            }
        }
    }

    fn owner(&self, ip: IpAddr) -> Option<usize> {
        self.peers
            .iter()
            .enumerate()
            .flat_map(|(i, p)| p.allowed.iter().map(move |n| (i, *n)))
            .filter(|(_, n)| contains(*n, ip))
            .max_by_key(|(_, n)| n.1)
            .map(|(i, _)| i)
    }

    /// Encrypts an IP packet for whichever peer owns its destination.
    async fn send_ip(&mut self, packet: &[u8]) {
        let Some(dst) = parse_ip(packet).map(|ip| ip.dst) else {
            return;
        };
        let Some(index) = self.owner(dst) else { return };
        let mut scratch = std::mem::take(&mut self.scratch);
        if scratch.len() < packet.len() + WG_OVERHEAD.max(148) {
            scratch.resize(packet.len() + 148, 0);
        }
        let len = match self.peers[index].tunn.encapsulate(packet, &mut scratch) {
            TunnResult::WriteToNetwork(out) => out.len(),
            _ => 0,
        };
        self.scratch = scratch;
        if len > 0 {
            self.send_to_peer(index, len).await;
        }
    }

    fn is_gateway(&self, ip: IpAddr) -> bool {
        ip == IpAddr::V4(self.gateway4) || ip == IpAddr::V6(self.gateway6)
    }

    fn routable(&self, ip: IpAddr) -> bool {
        let special = match ip {
            IpAddr::V4(v) => v.is_unspecified() || v.is_multicast() || v.is_broadcast(),
            IpAddr::V6(v) => v.is_unspecified() || v.is_multicast() || v.to_ipv4_mapped().is_some(),
        };
        !special && (self.allow_loopback || !ip.is_loopback())
    }

    /// Handles one decrypted packet from a peer.
    async fn inbound(&mut self, from: usize, packet: Vec<u8>) {
        let Some(ip) = parse_ip(&packet) else { return };
        let (src, dst, proto) = (ip.src, ip.dst, ip.proto);
        if !self.is_gateway(dst) {
            if let Some(owner) = self.owner(dst) {
                // Device-to-device traffic stays inside the tunnel.
                if owner != from {
                    self.send_ip(&packet).await;
                }
                return;
            }
            if !self.routable(dst) {
                return;
            }
        }
        let payload = ip.payload;
        match proto {
            TCP => {
                let (start, end) = (ip.offset, ip.offset + payload.len());
                self.tcp(src, dst, start..end, packet)
            }
            UDP => {
                if payload.len() < 8 {
                    return;
                }
                let sport = u16::from_be_bytes([payload[0], payload[1]]);
                let dport = u16::from_be_bytes([payload[2], payload[3]]);
                let len = u16::from_be_bytes([payload[4], payload[5]]) as usize;
                if len < 8 || len > payload.len() || dport == 0 {
                    return;
                }
                let shown = SocketAddr::new(dst, dport);
                let Some(to) = self.translate(shown) else {
                    return;
                };
                let data = payload[8..len].to_vec();
                self.udp_out(SocketAddr::new(src, sport), Datagram { to, shown, data });
            }
            ICMP | ICMP6 => {
                let request = if proto == ICMP { 8 } else { 128 };
                if payload.len() < 8 || payload[0] != request || src.is_ipv4() != (proto == ICMP) {
                    return;
                }
                let id = [payload[4], payload[5]];
                if self.is_gateway(dst) {
                    if let Some(reply) = build_echo_reply(dst, src, id, payload) {
                        self.send_ip(&reply).await;
                    }
                } else {
                    self.ping(src, dst, id, payload.to_vec());
                }
            }
            _ => {}
        }
    }

    /// Maps the address a client dialled to the one actually contacted. Only DNS is
    /// served on the tunnel address itself.
    fn translate(&self, shown: SocketAddr) -> Option<SocketAddr> {
        if !self.is_gateway(shown.ip()) {
            return Some(shown);
        }
        if shown.port() == 53 {
            self.resolvers.read().ok()?.first().copied()
        } else {
            None
        }
    }

    fn udp_out(&mut self, client: SocketAddr, datagram: Datagram) {
        let key = (client, datagram.to.is_ipv4());
        if let Some(flow) = self.udp.get(&key) {
            match flow.try_send(datagram) {
                Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => return,
                Err(mpsc::error::TrySendError::Closed(datagram)) => {
                    self.udp.remove(&key);
                    return self.udp_out(client, datagram);
                }
            }
        }
        if self.udp.len() >= MAX_UDP {
            self.udp.retain(|_, flow| !flow.is_closed());
            if self.udp.len() >= MAX_UDP {
                return;
            }
        }
        let Ok(socket) = udp_out(&self.config, key.1) else {
            return;
        };
        let (tx, rx) = mpsc::channel(64);
        let _ = tx.try_send(datagram);
        self.udp.insert(key, tx);
        self.tasks.spawn(udp_flow(
            socket,
            client,
            rx,
            self.outbound.0.clone(),
            self.stats.clone(),
            self.mtu,
        ));
    }

    fn ping(&mut self, client: IpAddr, target: IpAddr, id: [u8; 2], message: Vec<u8>) {
        let key = (client, target, id);
        if let Some(flow) = self.pings.get(&key) {
            if !flow.is_closed() {
                let _ = flow.try_send(message);
                return;
            }
            self.pings.remove(&key);
        }
        if self.pings.len() >= MAX_PING {
            self.pings.retain(|_, flow| !flow.is_closed());
            if self.pings.len() >= MAX_PING {
                return;
            }
        }
        // Platforms without unprivileged ICMP sockets silently drop pings.
        let Ok(socket) = ping_socket(&self.config, target) else {
            return;
        };
        let (tx, rx) = mpsc::channel(8);
        let _ = tx.try_send(message);
        self.pings.insert(key, tx);
        self.tasks.spawn(ping_flow(
            socket,
            client,
            target,
            id,
            rx,
            self.outbound.0.clone(),
        ));
    }

    fn tcp(&mut self, src: IpAddr, dst: IpAddr, segment: Range<usize>, packet: Vec<u8>) {
        let header = &packet[segment];
        if header.len() < 20 {
            return;
        }
        let sport = u16::from_be_bytes([header[0], header[1]]);
        let dport = u16::from_be_bytes([header[2], header[3]]);
        let (syn, ack, rst) = (
            header[13] & 2 != 0,
            header[13] & 16 != 0,
            header[13] & 4 != 0,
        );
        let key = (SocketAddr::new(src, sport), SocketAddr::new(dst, dport));
        match self.flows.get(&key) {
            // A retransmitted SYN while the destination is still being dialled.
            Some(Flow::Connecting(_)) => {
                if rst {
                    self.flows.remove(&key);
                }
            }
            Some(Flow::Open(_)) => self.queue.rx.push_back(packet),
            None => {
                let target = if syn && !ack && dport != 0 && self.flows.len() < MAX_TCP {
                    self.translate(key.1)
                } else {
                    None
                };
                let Some(target) = target else {
                    // No socket matches, so the stack answers with a reset.
                    self.queue.rx.push_back(packet);
                    return;
                };
                self.flows.insert(key, Flow::Connecting(packet));
                let config = self.config.clone();
                let events = self.events.0.clone();
                self.tasks.spawn(async move {
                    let result = match timeout(CONNECT, connect_one(target, config)).await {
                        Ok(result) => result,
                        Err(e) => Err(e.into()),
                    };
                    let _ = events.send(Event::Connected(key, result)).await;
                });
            }
        }
    }

    fn event(&mut self, event: Event) {
        let Event::Connected(key, result) = event;
        let Some(Flow::Connecting(syn)) = self.flows.remove(&key) else {
            return;
        };
        let Ok(stream) = result else {
            self.stats.failures.fetch_add(1, Ordering::Relaxed);
            // Replaying the SYN without a listener makes the stack refuse it.
            self.queue.rx.push_back(syn);
            return;
        };
        let mut socket = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; TCP_BUFFER]),
            tcp::SocketBuffer::new(vec![0; TCP_BUFFER]),
        );
        socket.set_nagle_enabled(!self.config.tcp_nodelay);
        // Give up on clients that vanish with data still queued for them.
        socket.set_timeout(Some(smoltcp::time::Duration::from_secs(60)));
        let endpoint = IpListenEndpoint {
            addr: Some(key.1.ip().into()),
            port: key.1.port(),
        };
        if socket.listen(endpoint).is_err() {
            return;
        }
        let handle = self.sockets.add(socket);
        // Accept this SYN right away so no other SYN can claim the listener.
        self.queue.rx.push_back(syn);
        let now = self.now();
        self.iface.poll(now, &mut self.queue, &mut self.sockets);
        let socket = self.sockets.get::<tcp::Socket>(handle);
        let accepted = socket
            .remote_endpoint()
            .is_some_and(|e| IpAddr::from(e.addr) == key.0.ip() && e.port == key.0.port());
        if !accepted {
            self.sockets.remove(handle);
            return;
        }
        let (up, up_rx) = mpsc::channel(8);
        let (down_tx, down) = mpsc::channel(8);
        let task = self
            .tasks
            .spawn(relay(stream, up_rx, down_tx, self.wake.clone()));
        self.stats.active.fetch_add(1, Ordering::Relaxed);
        self.stats.total.fetch_add(1, Ordering::Relaxed);
        self.flows.insert(
            key,
            Flow::Open(Box::new(OpenFlow {
                handle,
                up: Some(up),
                down,
                pending: None,
                upstream_done: false,
                closing: false,
                last: Instant::now(),
                task,
            })),
        );
    }

    /// Runs the TCP stack, shuttles bytes to and from relay tasks and sends
    /// whatever the stack produced.
    async fn poll(&mut self, mut pump: bool) {
        for _ in 0..8 {
            let now = self.now();
            if self.iface.poll(now, &mut self.queue, &mut self.sockets)
                == PollResult::SocketStateChanged
            {
                pump = true;
            }
            let moved = pump && self.pump();
            while let Some(packet) = self.queue.tx.pop_front() {
                self.send_ip(&packet).await;
            }
            if !moved {
                break;
            }
        }
    }

    fn pump(&mut self) -> bool {
        let mut moved = false;
        let mut finished = Vec::new();
        for (key, flow) in &mut self.flows {
            let Flow::Open(flow) = flow else { continue };
            let socket = self.sockets.get_mut::<tcp::Socket>(flow.handle);
            let before = moved;
            // Client to destination, bounded by the relay queue.
            while socket.can_recv() {
                let Some(up) = &flow.up else { break };
                match up.try_reserve() {
                    Ok(permit) => {
                        let data = socket
                            .recv(|bytes| {
                                let n = bytes.len().min(32768);
                                (n, bytes[..n].to_vec())
                            })
                            .unwrap_or_default();
                        if data.is_empty() {
                            break;
                        }
                        self.stats
                            .uploaded
                            .fetch_add(data.len() as u64, Ordering::Relaxed);
                        permit.send(data);
                        moved = true;
                    }
                    Err(mpsc::error::TrySendError::Full(())) => break,
                    Err(mpsc::error::TrySendError::Closed(())) => {
                        socket.abort();
                        break;
                    }
                }
            }
            // The client finished sending: half-close towards the destination.
            if flow.up.is_some() && !socket.may_recv() {
                flow.up = None;
                moved = true;
            }
            // Destination to client, bounded by the stack's send buffer.
            loop {
                if flow.pending.is_none() {
                    match flow.down.try_recv() {
                        Ok(Down::Data(data)) => flow.pending = Some((data, 0)),
                        Ok(Down::Eof) => flow.upstream_done = true,
                        Ok(Down::Reset) => socket.abort(),
                        Err(_) => {}
                    }
                }
                let Some((data, offset)) = &mut flow.pending else {
                    break;
                };
                let n = socket.send_slice(&data[*offset..]).unwrap_or(0);
                if n == 0 {
                    if !socket.may_send() {
                        flow.pending = None;
                    }
                    break;
                }
                self.stats.downloaded.fetch_add(n as u64, Ordering::Relaxed);
                *offset += n;
                moved = true;
                if *offset == data.len() {
                    flow.pending = None;
                }
            }
            if flow.upstream_done && flow.pending.is_none() && !flow.closing {
                socket.close();
                flow.closing = true;
                moved = true;
            }
            if moved != before {
                flow.last = Instant::now();
            } else if flow.last.elapsed() >= IDLE {
                socket.abort();
            }
            // A reset during the handshake puts the socket back into LISTEN, where it
            // would accept somebody else's SYN into this flow.
            let foreign = socket
                .remote_endpoint()
                .is_some_and(|e| IpAddr::from(e.addr) != key.0.ip() || e.port != key.0.port());
            if foreign || socket.state() == tcp::State::Listen {
                socket.abort();
            }
            if socket.state() == tcp::State::Closed {
                finished.push(*key);
            }
        }
        for key in finished {
            if let Some(Flow::Open(flow)) = self.flows.remove(&key) {
                // A reset was queued by abort(); let the stack emit it before the
                // socket goes away.
                let now = self.now();
                self.iface.poll(now, &mut self.queue, &mut self.sockets);
                self.sockets.remove(flow.handle);
                self.stats.active.fetch_sub(1, Ordering::Relaxed);
                // After an orderly close the relay drains and ends on its own.
                if !(flow.closing && flow.up.is_none()) {
                    flow.task.abort();
                }
                moved = true;
            }
        }
        moved
    }

    async fn timers(&mut self, second: bool) {
        for index in 0..self.peers.len() {
            let mut scratch = std::mem::take(&mut self.scratch);
            let len = match self.peers[index].tunn.update_timers(&mut scratch) {
                TunnResult::WriteToNetwork(out) => out.len(),
                _ => 0,
            };
            self.scratch = scratch;
            if len > 0 {
                self.send_to_peer(index, len).await;
            }
        }
        if !second {
            return;
        }
        self.limiter.reset_count();
        self.udp.retain(|_, flow| !flow.is_closed());
        self.pings.retain(|_, flow| !flow.is_closed());
        let status = self
            .peers
            .iter()
            .map(|peer| {
                let (handshake, sent, received, ..) = peer.tunn.stats();
                PeerStatus {
                    public_key: STANDARD.encode(peer.public),
                    endpoint: peer
                        .endpoint
                        .map(|e| canonical_addr(e).to_string())
                        .unwrap_or_default(),
                    handshake_age: handshake.map(|d| d.as_secs() as i64).unwrap_or(-1),
                    received: received as u64,
                    sent: sent as u64,
                }
            })
            .collect();
        if let Ok(mut guard) = self.stats.peers.lock() {
            *guard = status;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let open = self
            .flows
            .values()
            .filter(|f| matches!(f, Flow::Open(_)))
            .count();
        self.stats.active.fetch_sub(open as u64, Ordering::Relaxed);
        if let Ok(mut guard) = self.stats.peers.lock() {
            guard.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_match_whole_and_partial_bytes() {
        let net = cidr("10.77.0.0/23").unwrap();
        assert!(contains(net, "10.77.1.9".parse().unwrap()));
        assert!(!contains(net, "10.77.2.1".parse().unwrap()));
        assert!(contains(
            cidr("0.0.0.0/0").unwrap(),
            "8.8.8.8".parse().unwrap()
        ));
        assert!(contains(
            cidr("fd77::2").unwrap(),
            "fd77::2".parse().unwrap()
        ));
        assert!(!contains(
            cidr("fd77::2").unwrap(),
            "fd77::3".parse().unwrap()
        ));
        assert!(!contains(
            cidr("0.0.0.0/0").unwrap(),
            "::1".parse().unwrap()
        ));
        assert!(cidr("10.0.0.0/33").is_err());
    }

    #[test]
    fn built_packets_parse_back_with_valid_checksums() {
        for (src, dst) in [
            ("192.0.2.1:53", "10.77.0.2:4000"),
            ("[2001:db8::1]:53", "[fd77::2]:4000"),
        ] {
            let (src, dst): (SocketAddr, SocketAddr) = (src.parse().unwrap(), dst.parse().unwrap());
            let packet = build_udp(src, dst, b"hello").unwrap();
            let ip = parse_ip(&packet).unwrap();
            assert_eq!((ip.src, ip.dst, ip.proto), (src.ip(), dst.ip(), UDP));
            assert_eq!(&ip.payload[8..], b"hello");
            // A correct checksum sums to zero together with its pseudo header.
            assert_eq!(
                fold(sum(
                    ip.payload,
                    pseudo(ip.src, ip.dst, UDP, ip.payload.len())
                )),
                0
            );
            if src.is_ipv4() {
                assert_eq!(fold(sum(&packet[..20], 0)), 0);
            }
        }
    }

    #[test]
    fn fragments_and_truncated_packets_are_rejected() {
        let mut packet = build_udp(
            "192.0.2.1:1".parse().unwrap(),
            "192.0.2.2:2".parse().unwrap(),
            b"x",
        )
        .unwrap();
        assert!(parse_ip(&packet[..packet.len() - 1]).is_none());
        packet[6] = 0x20;
        assert!(parse_ip(&packet).is_none());
        assert!(parse_ip(&[]).is_none());
    }

    #[test]
    fn public_key_matches_rfc7748_vector() {
        assert_eq!(
            public_key("dwdtCnMYpX08FsFyUbJmRd9ML4frwJkqsXf7pR25LCo=").unwrap(),
            "hSDwCYkwp1R0i33ctD73Wg2/Og0mOBr066SpjqqbTmo="
        );
        assert!(public_key("short").is_err());
    }
}
