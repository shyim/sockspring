//! End-to-end tests: a real WireGuard client (boringtun) with its own TCP stack
//! (smoltcp) talks to the server over loopback UDP.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use boringtun::{
    noise::{Tunn, TunnResult},
    x25519::{PublicKey, StaticSecret},
};
use smoltcp::{
    iface::{Interface, SocketSet},
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    socket::tcp,
    wire::{HardwareAddress, IpCidr},
};
use socks5_rs::{
    serve,
    wg::{WgConfig, WgPeer},
    Config, Stats,
};
use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    sync::oneshot,
    time::timeout,
};

const GATEWAY4: Ipv4Addr = Ipv4Addr::new(10, 77, 0, 1);
const CLIENT4: Ipv4Addr = Ipv4Addr::new(10, 77, 0, 2);
const GATEWAY6: &str = "fd77:736f:636b::1";
const CLIENT6: &str = "fd77:736f:636b::2";

fn secret(seed: u8) -> StaticSecret {
    StaticSecret::from([seed; 32])
}
fn public(seed: u8) -> String {
    STANDARD.encode(PublicKey::from(&secret(seed)).as_bytes())
}
fn peer(seed: u8, host: u8) -> WgPeer {
    WgPeer {
        public_key: public(seed),
        preshared_key: String::new(),
        allowed_ips: vec![
            format!("10.77.0.{host}/32"),
            format!("fd77:736f:636b::{host}/128"),
        ],
    }
}

struct Server {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    stats: Arc<Stats>,
}
impl Server {
    async fn new(peers: Vec<WgPeer>, dns: Vec<String>) -> Self {
        let reservation = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = reservation.local_addr().unwrap();
        drop(reservation);
        let config = Config {
            socks: false,
            wireguard: WgConfig {
                enabled: true,
                listen: "127.0.0.1".into(),
                port: addr.port(),
                private_key: STANDARD.encode(secret(1).to_bytes()),
                peers,
                dns,
                allow_loopback: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let (stop, rx) = oneshot::channel();
        let (ready, wait) = std::sync::mpsc::sync_channel(1);
        let stats = Arc::new(Stats::default());
        let task = tokio::spawn(serve(config, stats.clone(), rx, ready));
        loop {
            match wait.try_recv() {
                Ok(result) => {
                    result.unwrap();
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    tokio::time::sleep(Duration::from_millis(1)).await
                }
                Err(e) => panic!("{e}"),
            }
        }
        Self {
            addr,
            stop: Some(stop),
            task,
            stats,
        }
    }
    fn stats(&self) -> serde_json::Value {
        serde_json::from_str(&self.stats.json()).unwrap()
    }
    async fn shutdown(mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        (&mut self.task).await.unwrap().unwrap();
        assert_eq!(self.stats()["running"], false);
        assert_eq!(self.stats()["active"], 0);
    }
}

struct Client {
    tunn: Tunn,
    socket: UdpSocket,
}
impl Client {
    async fn new(server: &Server, seed: u8, preshared: Option<[u8; 32]>) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket.connect(server.addr).await.unwrap();
        let mut client = Self {
            tunn: Tunn::new(
                secret(seed),
                PublicKey::from(&secret(1)),
                preshared,
                None,
                7,
                None,
            ),
            socket,
        };
        client.handshake().await;
        client
    }
    /// Tries to establish a session. Returns whether the server answered.
    async fn handshake(&mut self) -> bool {
        let mut out = vec![0; 256];
        if let TunnResult::WriteToNetwork(data) =
            self.tunn.format_handshake_initiation(&mut out, false)
        {
            self.socket.send(data).await.unwrap();
        }
        let mut datagram = vec![0; 256];
        let Ok(Ok(n)) = timeout(Duration::from_millis(300), self.socket.recv(&mut datagram)).await
        else {
            return false;
        };
        match self.tunn.decapsulate(None, &datagram[..n], &mut out) {
            TunnResult::WriteToNetwork(data) => {
                self.socket.send(data).await.unwrap();
                true
            }
            _ => false,
        }
    }
    async fn send(&mut self, packet: &[u8]) {
        let mut out = vec![0; packet.len() + 256];
        if let TunnResult::WriteToNetwork(data) = self.tunn.encapsulate(packet, &mut out) {
            self.socket.send(data).await.unwrap();
        }
    }
    /// Returns the next decrypted packet, completing the handshake on the way.
    async fn recv(&mut self, wait: Duration) -> Option<Vec<u8>> {
        let mut datagram = vec![0; 70000];
        let mut out = vec![0; 70000];
        loop {
            let n = timeout(wait, self.socket.recv(&mut datagram))
                .await
                .ok()?
                .ok()?;
            match self.tunn.decapsulate(None, &datagram[..n], &mut out) {
                TunnResult::WriteToNetwork(data) => {
                    self.socket.send(data).await.unwrap();
                    let mut more = vec![0; 70000];
                    while let TunnResult::WriteToNetwork(data) =
                        self.tunn.decapsulate(None, &[], &mut more)
                    {
                        self.socket.send(data).await.unwrap();
                    }
                }
                TunnResult::WriteToTunnelV4(packet, _) | TunnResult::WriteToTunnelV6(packet, _) => {
                    return Some(packet.to_vec())
                }
                _ => {}
            }
        }
    }
}

fn checksum(parts: &[&[u8]]) -> u16 {
    let mut acc = 0u32;
    for part in parts {
        for chunk in part.chunks(2) {
            acc += u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]) as u32;
        }
    }
    while acc >> 16 != 0 {
        acc = (acc & 0xffff) + (acc >> 16);
    }
    !(acc as u16)
}
fn ipv4(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![0x45, 0];
    p.extend(((20 + payload.len()) as u16).to_be_bytes());
    p.extend([0, 0, 0x40, 0, 64, proto, 0, 0]);
    p.extend(src.octets());
    p.extend(dst.octets());
    let check = checksum(&[&p]);
    p[10..12].copy_from_slice(&check.to_be_bytes());
    p.extend(payload);
    p
}
fn ipv6(src: Ipv6Addr, dst: Ipv6Addr, proto: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![0x60, 0, 0, 0];
    p.extend((payload.len() as u16).to_be_bytes());
    p.extend([proto, 64]);
    p.extend(src.octets());
    p.extend(dst.octets());
    p.extend(payload);
    p
}
fn udp(sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend(sport.to_be_bytes());
    p.extend(dport.to_be_bytes());
    p.extend(((8 + payload.len()) as u16).to_be_bytes());
    // Unverified by the server: WireGuard already authenticates the packet.
    p.extend([0, 0]);
    p.extend(payload);
    p
}
/// Splits an IPv4/IPv6 UDP packet into (source, destination, payload).
fn parse_udp(p: &[u8]) -> (SocketAddr, SocketAddr, Vec<u8>) {
    let (src, dst, body): (IpAddr, IpAddr, &[u8]) = if p[0] >> 4 == 4 {
        assert_eq!(p[9], 17);
        assert_eq!(checksum(&[&p[..20]]), 0, "IPv4 header checksum");
        let src = Ipv4Addr::new(p[12], p[13], p[14], p[15]);
        let dst = Ipv4Addr::new(p[16], p[17], p[18], p[19]);
        let pseudo = [&[0, 17][..], &((p.len() - 20) as u16).to_be_bytes()].concat();
        assert_eq!(
            checksum(&[&src.octets(), &dst.octets(), &pseudo, &p[20..]]),
            0,
            "UDP checksum"
        );
        (src.into(), dst.into(), &p[20..])
    } else {
        assert_eq!(p[6], 17);
        let src = Ipv6Addr::from(<[u8; 16]>::try_from(&p[8..24]).unwrap());
        let dst = Ipv6Addr::from(<[u8; 16]>::try_from(&p[24..40]).unwrap());
        let pseudo = [&[0, 17][..], &((p.len() - 40) as u16).to_be_bytes()].concat();
        assert_eq!(
            checksum(&[&src.octets(), &dst.octets(), &pseudo, &p[40..]]),
            0,
            "UDP checksum"
        );
        (src.into(), dst.into(), &p[40..])
    };
    let sport = u16::from_be_bytes([body[0], body[1]]);
    let dport = u16::from_be_bytes([body[2], body[3]]);
    (
        SocketAddr::new(src, sport),
        SocketAddr::new(dst, dport),
        body[8..].to_vec(),
    )
}

#[derive(Default)]
struct Queue {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
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
        caps.max_transmission_unit = 1420;
        caps
    }
}

/// The client's TCP stack with one socket.
struct Stack {
    iface: Interface,
    queue: Queue,
    sockets: SocketSet<'static>,
    handle: smoltcp::iface::SocketHandle,
    started: Instant,
}
impl Stack {
    fn connect(target: SocketAddr, local_port: u16) -> Self {
        let mut queue = Queue::default();
        let mut iface = Interface::new(
            smoltcp::iface::Config::new(HardwareAddress::Ip),
            &mut queue,
            smoltcp::time::Instant::ZERO,
        );
        iface.update_ip_addrs(|addrs| {
            addrs.push(IpCidr::new(CLIENT4.into(), 32)).unwrap();
            addrs
                .push(IpCidr::new(
                    CLIENT6.parse::<Ipv6Addr>().unwrap().into(),
                    128,
                ))
                .unwrap();
        });
        iface.routes_mut().add_default_ipv4_route(GATEWAY4).unwrap();
        iface
            .routes_mut()
            .add_default_ipv6_route(GATEWAY6.parse().unwrap())
            .unwrap();
        let mut socket = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; 65536]),
            tcp::SocketBuffer::new(vec![0; 65536]),
        );
        socket.connect(iface.context(), target, local_port).unwrap();
        let mut sockets = SocketSet::new(Vec::new());
        let handle = sockets.add(socket);
        Self {
            iface,
            queue,
            sockets,
            handle,
            started: Instant::now(),
        }
    }
    /// Runs the stack until `done` reports true. Panics after ten seconds.
    async fn drive(&mut self, client: &mut Client, mut done: impl FnMut(&mut tcp::Socket) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let now =
                smoltcp::time::Instant::from_micros(self.started.elapsed().as_micros() as i64);
            self.iface.poll(now, &mut self.queue, &mut self.sockets);
            if done(self.sockets.get_mut::<tcp::Socket>(self.handle)) {
                self.iface.poll(now, &mut self.queue, &mut self.sockets);
                while let Some(packet) = self.queue.tx.pop_front() {
                    client.send(&packet).await;
                }
                return;
            }
            while let Some(packet) = self.queue.tx.pop_front() {
                client.send(&packet).await;
            }
            if let Some(packet) = client.recv(Duration::from_millis(5)).await {
                self.queue.rx.push_back(packet);
            }
            assert!(Instant::now() < deadline, "TCP exchange timed out");
        }
    }
}

async fn wait_for(mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached");
}

#[tokio::test]
async fn tcp_transfer_half_close_and_counters() {
    let server = Server::new(vec![peer(2, 2)], vec![]).await;
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = target.local_addr().unwrap();
    // More than the stack buffers hold, so flow control is exercised both ways.
    let payload: Vec<u8> = (0..600_000u32).map(|i| (i % 251) as u8).collect();
    let expected = payload.clone();
    let echo = tokio::spawn(async move {
        let (mut s, _) = target.accept().await.unwrap();
        let mut input = Vec::new();
        s.read_to_end(&mut input).await.unwrap();
        assert_eq!(input, expected);
        s.write_all(&input).await.unwrap();
    });
    let mut client = Client::new(&server, 2, None).await;
    let mut stack = Stack::connect(address, 40000);
    let mut sent = 0;
    stack
        .drive(&mut client, |socket| {
            if socket.may_send() && sent < payload.len() {
                sent += socket.send_slice(&payload[sent..]).unwrap();
            }
            if sent == payload.len() {
                socket.close();
                return true;
            }
            false
        })
        .await;
    let mut received = Vec::new();
    stack
        .drive(&mut client, |socket| {
            while socket.can_recv() {
                socket
                    .recv(|bytes| {
                        received.extend_from_slice(bytes);
                        (bytes.len(), ())
                    })
                    .unwrap();
            }
            !socket.may_recv()
        })
        .await;
    assert_eq!(received, payload);
    echo.await.unwrap();
    stack.drive(&mut client, |socket| !socket.is_active()).await;
    wait_for(|| server.stats()["active"] == 0).await;
    let stats = server.stats();
    assert_eq!(stats["uploaded"], 600_000);
    assert_eq!(stats["downloaded"], 600_000);
    assert_eq!(stats["total"], 1);
    assert_eq!(stats["failures"], 0);
    server.shutdown().await;
}

#[tokio::test]
async fn tcp_over_ipv6_inside_the_tunnel() {
    // Hosts without IPv6 cannot dial the destination at all.
    let Ok(target) = TcpListener::bind("[::1]:0").await else {
        return;
    };
    let server = Server::new(vec![peer(2, 2)], vec![]).await;
    let address = target.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = target.accept().await.unwrap();
        let mut b = [0; 5];
        s.read_exact(&mut b).await.unwrap();
        s.write_all(&b).await.unwrap();
    });
    let mut client = Client::new(&server, 2, None).await;
    let mut stack = Stack::connect(address, 40001);
    let mut sent = false;
    let mut received = Vec::new();
    stack
        .drive(&mut client, |socket| {
            if socket.may_send() && !sent {
                socket.send_slice(b"hello").unwrap();
                sent = true;
            }
            while socket.can_recv() {
                socket
                    .recv(|bytes| {
                        received.extend_from_slice(bytes);
                        (bytes.len(), ())
                    })
                    .unwrap();
            }
            received.len() == 5
        })
        .await;
    assert_eq!(received, b"hello");
    server.shutdown().await;
}

#[tokio::test]
async fn refused_destination_resets_the_client() {
    let server = Server::new(vec![peer(2, 2)], vec![]).await;
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    let mut client = Client::new(&server, 2, None).await;
    let mut stack = Stack::connect(address, 40002);
    stack
        .drive(&mut client, |socket| socket.state() == tcp::State::Closed)
        .await;
    assert_eq!(server.stats()["failures"], 1);
    assert_eq!(server.stats()["total"], 0);
    server.shutdown().await;
}

/// A bare IPv4 TCP segment with a valid checksum and no options.
fn tcp_segment(sport: u16, dst: SocketAddr, seq: u32, ack: u32, flags: u8) -> Vec<u8> {
    let IpAddr::V4(target) = dst.ip() else {
        panic!("IPv4 only")
    };
    let mut t = Vec::new();
    t.extend(sport.to_be_bytes());
    t.extend(dst.port().to_be_bytes());
    t.extend(seq.to_be_bytes());
    t.extend(ack.to_be_bytes());
    t.extend([0x50, flags, 0xff, 0xff, 0, 0, 0, 0]);
    let check = checksum(&[&CLIENT4.octets(), &target.octets(), &[0, 6, 0, 20], &t]);
    t[16..18].copy_from_slice(&check.to_be_bytes());
    ipv4(CLIENT4, target, 6, &t)
}

#[tokio::test]
async fn reset_during_handshake_releases_the_flow() {
    let server = Server::new(vec![peer(2, 2)], vec![]).await;
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = target.local_addr().unwrap();
    let mut client = Client::new(&server, 2, None).await;
    // SYN, wait for the SYN-ACK, then abandon the attempt with a reset.
    client.send(&tcp_segment(41000, address, 1000, 0, 2)).await;
    let reply = client.recv(Duration::from_secs(5)).await.unwrap();
    assert_eq!(reply[20 + 13] & 0x12, 0x12, "expected SYN-ACK");
    let (mut first, _) = target.accept().await.unwrap();
    client.send(&tcp_segment(41000, address, 1001, 0, 4)).await;
    // The destination connection is dropped rather than handed to the next caller.
    let mut b = [0; 1];
    assert!(matches!(
        timeout(Duration::from_secs(5), first.read(&mut b)).await,
        Ok(Ok(0) | Err(_))
    ));
    wait_for(|| server.stats()["active"] == 0).await;
    // A later connection to the same destination gets a connection of its own.
    let mut stack = Stack::connect(address, 41001);
    stack
        .drive(&mut client, |socket| {
            if socket.may_send() {
                socket.send_slice(b"x").unwrap();
                return true;
            }
            false
        })
        .await;
    let (mut second, _) = target.accept().await.unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                r = second.read(&mut b) => break assert_eq!(r.unwrap(), 1),
                _ = stack.drive(&mut client, |_| false) => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(server.stats()["total"], 2);
    server.shutdown().await;
}

#[tokio::test]
async fn udp_round_trip_and_source_filtering() {
    let server = Server::new(vec![peer(2, 2)], vec![]).await;
    let target = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = target.local_addr().unwrap();
    let mut client = Client::new(&server, 2, None).await;
    let packet = ipv4(
        CLIENT4,
        Ipv4Addr::LOCALHOST,
        17,
        &udp(5000, address.port(), b"ping"),
    );
    client.send(&packet).await;
    let mut b = [0; 16];
    let (n, relay) = timeout(Duration::from_secs(5), target.recv_from(&mut b))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&b[..n], b"ping");
    // Only the contacted destination may answer.
    stranger.send_to(b"spoof", relay).await.unwrap();
    target.send_to(b"pong", relay).await.unwrap();
    let reply = client.recv(Duration::from_secs(5)).await.unwrap();
    let (src, dst, data) = parse_udp(&reply);
    assert_eq!(src, address);
    assert_eq!(dst, SocketAddr::new(CLIENT4.into(), 5000));
    assert_eq!(data, b"pong");
    assert!(client.recv(Duration::from_millis(200)).await.is_none());
    let stats = server.stats();
    assert_eq!(stats["uploaded"], 4);
    assert_eq!(stats["downloaded"], 4);
    server.shutdown().await;
}

#[tokio::test]
async fn udp_over_ipv6_inside_the_tunnel() {
    let Ok(target) = UdpSocket::bind("[::1]:0").await else {
        return;
    };
    let server = Server::new(vec![peer(2, 2)], vec![]).await;
    let address = target.local_addr().unwrap();
    let mut client = Client::new(&server, 2, None).await;
    let source: Ipv6Addr = CLIENT6.parse().unwrap();
    client
        .send(&ipv6(
            source,
            Ipv6Addr::LOCALHOST,
            17,
            &udp(5001, address.port(), b"six"),
        ))
        .await;
    let mut b = [0; 16];
    let (n, relay) = timeout(Duration::from_secs(5), target.recv_from(&mut b))
        .await
        .unwrap()
        .unwrap();
    target.send_to(&b[..n], relay).await.unwrap();
    let (src, dst, data) = parse_udp(&client.recv(Duration::from_secs(5)).await.unwrap());
    assert_eq!(src, address);
    assert_eq!(dst, SocketAddr::new(source.into(), 5001));
    assert_eq!(data, b"six");
    server.shutdown().await;
}

#[tokio::test]
async fn dns_on_the_tunnel_address_uses_the_configured_resolver() {
    let resolver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server = Server::new(
        vec![peer(2, 2)],
        vec![resolver.local_addr().unwrap().to_string()],
    )
    .await;
    let mut client = Client::new(&server, 2, None).await;
    client
        .send(&ipv4(CLIENT4, GATEWAY4, 17, &udp(5353, 53, b"query")))
        .await;
    let mut b = [0; 16];
    let (n, relay) = timeout(Duration::from_secs(5), resolver.recv_from(&mut b))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&b[..n], b"query");
    resolver.send_to(b"answer", relay).await.unwrap();
    let (src, dst, data) = parse_udp(&client.recv(Duration::from_secs(5)).await.unwrap());
    // The client sees the answer from the address it asked.
    assert_eq!(src, SocketAddr::new(GATEWAY4.into(), 53));
    assert_eq!(dst, SocketAddr::new(CLIENT4.into(), 5353));
    assert_eq!(data, b"answer");
    // An IPv6 client is served by the same resolver, whatever its family.
    let (source, gateway): (Ipv6Addr, Ipv6Addr) =
        (CLIENT6.parse().unwrap(), GATEWAY6.parse().unwrap());
    client
        .send(&ipv6(source, gateway, 17, &udp(5354, 53, b"six")))
        .await;
    let (n, relay) = timeout(Duration::from_secs(5), resolver.recv_from(&mut b))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&b[..n], b"six");
    resolver.send_to(b"reply", relay).await.unwrap();
    let (src, dst, data) = parse_udp(&client.recv(Duration::from_secs(5)).await.unwrap());
    assert_eq!(src, SocketAddr::new(gateway.into(), 53));
    assert_eq!(dst, SocketAddr::new(source.into(), 5354));
    assert_eq!(data, b"reply");
    // The resolver can be swapped while running, as Android does on network changes.
    let replacement = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socks5_rs::wg::set_dns(&replacement.local_addr().unwrap().to_string());
    client
        .send(&ipv4(CLIENT4, GATEWAY4, 17, &udp(5353, 53, b"again")))
        .await;
    let mut c = [0; 16];
    tokio::select! {
        r = replacement.recv_from(&mut c) => assert_eq!(&c[..r.unwrap().0], b"again"),
        // Another test's server may have registered since; the old resolver is then still valid.
        r = resolver.recv_from(&mut b) => assert_eq!(&b[..r.unwrap().0], b"again"),
        _ = tokio::time::sleep(Duration::from_secs(5)) => panic!("query was not forwarded"),
    }
    // Nothing else is served on the tunnel address.
    client
        .send(&ipv4(CLIENT4, GATEWAY4, 17, &udp(5353, 80, b"nope")))
        .await;
    assert!(client.recv(Duration::from_millis(200)).await.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn tunnel_address_answers_ping() {
    let server = Server::new(vec![peer(2, 2)], vec![]).await;
    let mut client = Client::new(&server, 2, None).await;
    let mut echo = vec![8, 0, 0, 0, 0x12, 0x34, 0, 1];
    echo.extend(b"abcdefgh");
    let check = checksum(&[&echo]);
    echo[2..4].copy_from_slice(&check.to_be_bytes());
    client.send(&ipv4(CLIENT4, GATEWAY4, 1, &echo)).await;
    let reply = client.recv(Duration::from_secs(5)).await.unwrap();
    assert_eq!(reply[9], 1);
    assert_eq!(&reply[12..16], &GATEWAY4.octets());
    assert_eq!(&reply[16..20], &CLIENT4.octets());
    let icmp = &reply[20..];
    assert_eq!(icmp[0], 0);
    assert_eq!(&icmp[4..6], &[0x12, 0x34]);
    assert_eq!(&icmp[8..], b"abcdefgh");
    assert_eq!(checksum(&[icmp]), 0);

    let (src, dst): (Ipv6Addr, Ipv6Addr) = (CLIENT6.parse().unwrap(), GATEWAY6.parse().unwrap());
    let mut echo = vec![128, 0, 0, 0, 0x56, 0x78, 0, 1];
    echo.extend(b"v6");
    client.send(&ipv6(src, dst, 58, &echo)).await;
    let reply = client.recv(Duration::from_secs(5)).await.unwrap();
    assert_eq!(reply[6], 58);
    let icmp = &reply[40..];
    assert_eq!(icmp[0], 129);
    assert_eq!(&icmp[4..6], &[0x56, 0x78]);
    let pseudo = [&[0, 58][..], &(icmp.len() as u16).to_be_bytes()].concat();
    assert_eq!(checksum(&[&dst.octets(), &src.octets(), &pseudo, icmp]), 0);
    server.shutdown().await;
}

#[tokio::test]
async fn pings_to_other_hosts_are_relayed() {
    // Needs unprivileged ICMP sockets (always on for Android; a sysctl elsewhere).
    if socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::ICMPV4),
    )
    .is_err()
    {
        return;
    }
    let server = Server::new(vec![peer(2, 2)], vec![]).await;
    let mut client = Client::new(&server, 2, None).await;
    let mut echo = vec![8, 0, 0, 0, 0xab, 0xcd, 0, 7];
    echo.extend(b"relayed");
    let check = checksum(&[&echo]);
    echo[2..4].copy_from_slice(&check.to_be_bytes());
    client
        .send(&ipv4(CLIENT4, Ipv4Addr::LOCALHOST, 1, &echo))
        .await;
    let reply = client.recv(Duration::from_secs(5)).await.unwrap();
    assert_eq!(&reply[12..16], &Ipv4Addr::LOCALHOST.octets());
    assert_eq!(&reply[16..20], &CLIENT4.octets());
    let icmp = &reply[20..];
    // The kernel picked its own identifier; the client must see the original.
    assert_eq!(icmp[0], 0);
    assert_eq!(&icmp[4..8], &[0xab, 0xcd, 0, 7]);
    assert_eq!(&icmp[8..], b"relayed");
    assert_eq!(checksum(&[icmp]), 0);
    server.shutdown().await;
}

#[tokio::test]
async fn unknown_keys_wrong_secrets_and_spoofed_sources_are_ignored() {
    let mut with_secret = peer(3, 3);
    with_secret.preshared_key = STANDARD.encode([9; 32]);
    let server = Server::new(vec![peer(2, 2), with_secret], vec![]).await;
    let target = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = target.local_addr().unwrap().port();
    let datagram = |source: Ipv4Addr| ipv4(source, Ipv4Addr::LOCALHOST, 17, &udp(5000, port, b"x"));
    let mut b = [0; 4];

    // A key the server was never given.
    let mut unknown = Client::new(&server, 42, None).await;
    unknown.send(&datagram(CLIENT4)).await;
    assert!(unknown.recv(Duration::from_millis(300)).await.is_none());
    // The right key with the wrong preshared secret.
    let mut wrong = Client::new(&server, 3, Some([8; 32])).await;
    wrong.send(&datagram(Ipv4Addr::new(10, 77, 0, 3))).await;
    assert!(wrong.recv(Duration::from_millis(300)).await.is_none());
    // A valid device using an address assigned to another one.
    let mut spoof = Client::new(&server, 2, None).await;
    spoof.send(&datagram(Ipv4Addr::new(10, 77, 0, 3))).await;
    assert!(spoof.recv(Duration::from_millis(300)).await.is_none());
    assert!(
        timeout(Duration::from_millis(300), target.recv_from(&mut b))
            .await
            .is_err()
    );

    // The same devices work once they follow the rules.
    spoof.send(&datagram(CLIENT4)).await;
    let mut right = Client::new(&server, 3, Some([9; 32])).await;
    right.send(&datagram(Ipv4Addr::new(10, 77, 0, 3))).await;
    for _ in 0..2 {
        timeout(Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    r = target.recv_from(&mut b) => break r.unwrap(),
                    _ = spoof.recv(Duration::from_millis(50)) => {}
                    _ = right.recv(Duration::from_millis(50)) => {}
                }
            }
        })
        .await
        .unwrap();
    }
    server.shutdown().await;
}

#[tokio::test]
async fn devices_reach_each_other_and_report_status() {
    let server = Server::new(vec![peer(2, 2), peer(3, 3)], vec![]).await;
    let mut first = Client::new(&server, 2, None).await;
    let mut second = Client::new(&server, 3, None).await;
    let other = Ipv4Addr::new(10, 77, 0, 3);
    // The second device must have spoken once so the server knows where it is.
    second
        .send(&ipv4(other, GATEWAY4, 17, &udp(1, 9, b"hi")))
        .await;
    assert!(second.recv(Duration::from_millis(300)).await.is_none());
    let packet = ipv4(CLIENT4, other, 17, &udp(7000, 7001, b"direct"));
    let mut forwarded = None;
    for _ in 0..20 {
        first.send(&packet).await;
        tokio::select! {
            p = second.recv(Duration::from_millis(250)) => if p.is_some() { forwarded = p; break },
            _ = first.recv(Duration::from_millis(250)) => {}
        }
    }
    assert_eq!(forwarded.unwrap(), packet);

    wait_for(|| {
        let peers = server.stats()["peers"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        peers.len() == 2
            && peers.iter().all(|p| {
                p["handshake_age"].as_i64().unwrap() >= 0
                    && p["endpoint"].as_str().unwrap().starts_with("127.0.0.1:")
                    && p["received"].as_u64().unwrap() > 0
            })
    })
    .await;
    assert_eq!(server.stats()["peers"][0]["public_key"], public(2));
    let stats = server.stats.clone();
    server.shutdown().await;
    let after: serde_json::Value = serde_json::from_str(&stats.json()).unwrap();
    assert_eq!(after["peers"].as_array().unwrap().len(), 0);
}

#[test]
fn configuration_validation() {
    let valid = Config {
        wireguard: WgConfig {
            enabled: true,
            private_key: STANDARD.encode([1; 32]),
            peers: vec![peer(2, 2)],
            ..Default::default()
        },
        ..Default::default()
    };
    valid.validate().unwrap();
    let broken = |change: fn(&mut Config)| {
        let mut config = valid.clone();
        change(&mut config);
        config.validate().is_err()
    };
    assert!(broken(|c| c.wireguard.private_key = "nope".into()));
    assert!(broken(|c| c.wireguard.port = 0));
    assert!(broken(|c| c.wireguard.mtu = 1000));
    assert!(broken(|c| c.wireguard.address4 = "fd77::1".into()));
    assert!(broken(|c| c.wireguard.peers.push(peer(2, 3))));
    assert!(broken(
        |c| c.wireguard.peers[0].allowed_ips = vec!["10.0.0.0/40".into()]
    ));
    assert!(broken(
        |c| c.wireguard.peers[0].preshared_key = "short".into()
    ));
    assert!(broken(|c| c.wireguard.dns = vec!["dns.example".into()]));
    assert!(broken(|c| {
        c.socks = false;
        c.wireguard.enabled = false;
    }));
    // A disabled tunnel is not validated, so stale settings never block SOCKS.
    assert!(!broken(|c| {
        c.wireguard.enabled = false;
        c.wireguard.private_key = String::new();
    }));
}

#[test]
fn engine_runs_both_servers_and_reports_occupied_ports() {
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let json = serde_json::json!({
        "listen": "127.0.0.1",
        "port": tcp.local_addr().unwrap().port(),
        "wireguard": {
            "enabled": true,
            "listen": "127.0.0.1",
            "port": udp.local_addr().unwrap().port(),
            "private_key": STANDARD.encode([1; 32]),
            "peers": [{"public_key": public(2), "allowed_ips": ["10.77.0.2/32"]}],
        },
    })
    .to_string();
    drop(tcp);
    // The WireGuard port is still taken, so nothing may keep running.
    assert!(socks5_rs::start(&json).is_err());
    drop(udp);
    socks5_rs::start(&json).unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&socks5_rs::snapshot()).unwrap();
    assert_eq!(snapshot["running"], true);
    socks5_rs::stop();
    socks5_rs::start(&json).unwrap();
    socks5_rs::stop();
}
