use socks5_rs::{serve, Config, Stats};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::oneshot,
};

struct Server {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    stats: Arc<Stats>,
}
impl Server {
    async fn new(mut config: Config) -> Self {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = reservation.local_addr().unwrap();
        drop(reservation);
        config.listen = "127.0.0.1".into();
        config.port = addr.port();
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
    async fn client(&self) -> TcpStream {
        let mut s = TcpStream::connect(self.addr).await.unwrap();
        s.write_all(&[5, 1, 0]).await.unwrap();
        let mut b = [0; 2];
        s.read_exact(&mut b).await.unwrap();
        assert_eq!(b, [5, 0]);
        s
    }
    async fn shutdown(mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        self.task.await.unwrap().unwrap();
        assert!(
            !serde_json::from_str::<serde_json::Value>(&self.stats.json()).unwrap()["running"]
                .as_bool()
                .unwrap()
        );
        std::net::TcpListener::bind(self.addr).unwrap();
    }
}
fn addr_bytes(a: SocketAddr) -> Vec<u8> {
    let mut b = vec![];
    match a.ip() {
        std::net::IpAddr::V4(ip) => {
            b.push(1);
            b.extend(ip.octets());
        }
        std::net::IpAddr::V6(ip) => {
            b.push(4);
            b.extend(ip.octets());
        }
    }
    b.extend(a.port().to_be_bytes());
    b
}
async fn response(s: &mut TcpStream) -> (u8, SocketAddr) {
    let mut h = [0; 4];
    s.read_exact(&mut h).await.unwrap();
    assert_eq!(h[0], 5);
    let ip = match h[3] {
        1 => {
            let mut b = [0; 4];
            s.read_exact(&mut b).await.unwrap();
            std::net::IpAddr::V4(b.into())
        }
        4 => {
            let mut b = [0; 16];
            s.read_exact(&mut b).await.unwrap();
            std::net::IpAddr::V6(b.into())
        }
        _ => panic!("Bad reply address"),
    };
    (h[1], SocketAddr::new(ip, s.read_u16().await.unwrap()))
}
async fn request(s: &mut TcpStream, cmd: u8, addr: SocketAddr) -> (u8, SocketAddr) {
    let mut b = vec![5, cmd, 0];
    b.extend(addr_bytes(addr));
    s.write_all(&b).await.unwrap();
    response(s).await
}

#[tokio::test]
async fn tcp_payload_half_close_stats_and_shutdown() {
    let server = Server::new(Config::default()).await;
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = target.local_addr().unwrap();
    let echo = tokio::spawn(async move {
        let (mut s, _) = target.accept().await.unwrap();
        let mut input = Vec::new();
        s.read_to_end(&mut input).await.unwrap();
        assert_eq!(input, vec![42; 131072]);
        s.write_all(&input).await.unwrap();
    });
    let mut s = server.client().await;
    assert_eq!(request(&mut s, 1, address).await.0, 0);
    s.write_all(&vec![42; 131072]).await.unwrap();
    s.shutdown().await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    assert_eq!(out, vec![42; 131072]);
    echo.await.unwrap();
    let stats: serde_json::Value = serde_json::from_str(&server.stats.json()).unwrap();
    assert_eq!(stats["uploaded"], 131072);
    assert_eq!(stats["downloaded"], 131072);
    server.shutdown().await;
}
#[tokio::test]
async fn authentication_required_bad_and_good_credentials() {
    let server = Server::new(Config {
        username: "user".into(),
        password: "secret".into(),
        ..Default::default()
    })
    .await;
    let mut s = TcpStream::connect(server.addr).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    assert_eq!(s.read_u16().await.unwrap(), 0x05ff);
    for (password, expected) in [(b"wrong!".as_slice(), 1), (b"secret".as_slice(), 0)] {
        let mut s = TcpStream::connect(server.addr).await.unwrap();
        s.write_all(&[5, 1, 2]).await.unwrap();
        assert_eq!(s.read_u16().await.unwrap(), 0x0502);
        let mut b = vec![1, 4];
        b.extend(b"user");
        b.push(6);
        b.extend(password);
        s.write_all(&b).await.unwrap();
        assert_eq!(s.read_u16().await.unwrap(), 0x0100 + expected);
        if expected == 0 {
            assert_eq!(
                request(&mut s, 2, "127.0.0.1:1".parse().unwrap()).await.0,
                7
            );
        }
    }
    server.shutdown().await;
}
#[tokio::test]
async fn invalid_addresses_and_unknown_commands_return_errors() {
    let server = Server::new(Config::default()).await;
    let mut s = server.client().await;
    s.write_all(&[5, 1, 0, 9]).await.unwrap();
    assert_eq!(response(&mut s).await.0, 8);
    let mut s = server.client().await;
    assert_eq!(
        request(&mut s, 2, "127.0.0.1:1".parse().unwrap()).await.0,
        7
    );
    let mut s = server.client().await;
    s.write_all(&[5, 1, 0, 3, 0]).await.unwrap();
    assert_eq!(response(&mut s).await.0, 8);
    server.shutdown().await;
}
#[tokio::test]
async fn domain_name_connect() {
    let server = Server::new(Config::default()).await;
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = target.local_addr().unwrap().port();
    let mut s = server.client().await;
    let mut request = vec![5, 1, 0, 3, 9];
    request.extend(b"localhost");
    request.extend(port.to_be_bytes());
    s.write_all(&request).await.unwrap();
    assert_eq!(response(&mut s).await.0, 0);
    server.shutdown().await;
}
#[tokio::test]
async fn udp_associate_payload_and_source_filtering() {
    let server = Server::new(Config::default()).await;
    let target = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let rogue = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut control = server.client().await;
    let (code, relay) = request(&mut control, 3, client.local_addr().unwrap()).await;
    assert_eq!(code, 0);
    let mut packet = vec![0, 0, 0];
    packet.extend(addr_bytes(target.local_addr().unwrap()));
    packet.extend(b"hello-udp");
    client.send_to(&packet, relay).await.unwrap();
    let mut buf = [0; 1024];
    let (n, peer) = target.recv_from(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"hello-udp");
    rogue.send_to(b"unsolicited", peer).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), client.recv_from(&mut buf))
            .await
            .is_err()
    );
    target.send_to(b"reply", peer).await.unwrap();
    let (n, _) = client.recv_from(&mut buf).await.unwrap();
    assert_eq!(&buf[n - 5..n], b"reply");
    assert_eq!(&buf[..3], &[0, 0, 0]);
    packet[2] = 1;
    client.send_to(&packet, relay).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), target.recv_from(&mut buf))
            .await
            .is_err()
    );
    drop(control);
    server.shutdown().await;
}
#[tokio::test]
async fn udp_over_tcp_split_frames() {
    let server = Server::new(Config::default()).await;
    let target = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut client = server.client().await;
    assert_eq!(
        request(&mut client, 5, "0.0.0.0:0".parse().unwrap())
            .await
            .0,
        0
    );
    let addr = addr_bytes(target.local_addr().unwrap());
    let mut frame = vec![0, 5, (addr.len() + 3) as u8];
    frame.extend(addr);
    frame.extend(b"hello");
    for chunk in frame.chunks(2) {
        client.write_all(chunk).await.unwrap();
        tokio::task::yield_now().await;
    }
    let mut buf = [0; 1024];
    let (n, peer) = target.recv_from(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"hello");
    target.send_to(b"world", peer).await.unwrap();
    let length = client.read_u16().await.unwrap();
    assert_eq!(length, 5);
    let header = client.read_u8().await.unwrap() as usize;
    let mut b = vec![0; header - 3 + length as usize];
    client.read_exact(&mut b).await.unwrap();
    assert_eq!(&b[b.len() - 5..], b"world");
    server.shutdown().await;
}
#[tokio::test]
async fn shutdown_closes_open_connections() {
    let server = Server::new(Config::default()).await;
    let mut client = server.client().await;
    server.shutdown().await;
    assert_eq!(
        client.read_u8().await.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
}
#[test]
fn configuration_validation() {
    assert!(Config::default().validate().is_ok());
    for json in [
        r#"{"workers":0}"#,
        r#"{"workers":33}"#,
        r#"{"port":0}"#,
        r#"{"username":"only"}"#,
        r#"{"udp_port":"9-2"}"#,
        r#"{"bind_ipv4":"::"}"#,
        r#"{"listen":"example.com"}"#,
    ] {
        assert!(
            serde_json::from_str::<Config>(json)
                .unwrap()
                .validate()
                .is_err(),
            "{json}"
        );
    }
}
#[test]
fn engine_lifecycle_and_bind_failure() {
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    let json = format!(r#"{{"listen":"127.0.0.1","port":{port}}}"#);
    assert!(socks5_rs::start(&json).is_err());
    drop(blocker);
    socks5_rs::start(&json).unwrap();
    assert!(socks5_rs::start(&json).is_err());
    socks5_rs::stop();
    socks5_rs::stop();
    socks5_rs::start(&json).unwrap();
    socks5_rs::stop();
}
#[tokio::test]
async fn ipv6_connect() {
    let target = match TcpListener::bind("[::1]:0").await {
        Ok(t) => t,
        Err(_) => return,
    };
    let server = Server::new(Config::default()).await;
    let mut s = server.client().await;
    assert_eq!(request(&mut s, 1, target.local_addr().unwrap()).await.0, 0);
    server.shutdown().await;
}
