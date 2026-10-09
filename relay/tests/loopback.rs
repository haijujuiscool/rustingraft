//! Real server and clients over 127.0.0.1.

use raft_relay::{
    read_frame, write_frame, ClientIncoming, ClientLink, Config, HostIncoming, Refusal,
    RelayClient, RelayError, Server, MAX_FRAME,
};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

fn start(config: Config) -> SocketAddr {
    let server = Server::bind(Config { bind: "127.0.0.1:0".parse().unwrap(), ..config }).unwrap();
    let addr = server.local_addr().unwrap();
    thread::spawn(move || -> () { server.run() });
    addr
}

fn wait<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for a message");
        thread::sleep(Duration::from_millis(2));
    }
}

fn refusal(result: Result<(u8, ClientLink), RelayError>) -> Refusal {
    match result {
        Err(RelayError::Refused(reason)) => reason,
        Err(RelayError::Io(e)) => panic!("expected a refusal, got {e}"),
        Ok((id, _)) => panic!("expected a refusal, joined as {id}"),
    }
}

fn data(bytes: &[u8]) -> ClientIncoming {
    ClientIncoming::Data(bytes.to_vec())
}

#[test]
fn host_and_two_clients_exchange_data() {
    let addr = start(Config::default());
    let (code, host) = RelayClient::host(addr).unwrap();
    assert!(raft_relay::is_valid_code(&code));

    let (id1, c1) = RelayClient::join(addr, &code).unwrap();
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Joined(id1));
    let (id2, c2) = RelayClient::join(addr, &format!(" {} ", code.to_lowercase())).unwrap();
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Joined(id2));
    assert_eq!((id1, id2), (1, 2));

    c1.send(b"from one").unwrap();
    let msg = wait(|| host.try_recv());
    assert_eq!(msg, HostIncoming::Data { from: id1, bytes: b"from one".to_vec() });
    c2.send(b"from two").unwrap();
    let msg = wait(|| host.try_recv());
    assert_eq!(msg, HostIncoming::Data { from: id2, bytes: b"from two".to_vec() });

    host.send_to(0, b"everyone").unwrap();
    assert_eq!(wait(|| c1.try_recv()), data(b"everyone"));
    assert_eq!(wait(|| c2.try_recv()), data(b"everyone"));

    // Targeted: c1's next message must be its own, proving it never saw c2's.
    host.send_to(id2, b"only two").unwrap();
    host.send_to(id1, b"only one").unwrap();
    assert_eq!(wait(|| c2.try_recv()), data(b"only two"));
    assert_eq!(wait(|| c1.try_recv()), data(b"only one"));

    let big = vec![0xAB; raft_relay::MAX_DATA];
    host.send_to(id1, &big).unwrap();
    assert_eq!(wait(|| c1.try_recv()), ClientIncoming::Data(big.clone()));
    c1.send(&big).unwrap();
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Data { from: id1, bytes: big });
    assert_eq!(c1.send(&[0; MAX_FRAME]).unwrap_err().kind(), ErrorKind::InvalidInput);

    drop(c1);
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Left(id1));

    drop(host);
    assert!(matches!(wait(|| c2.try_recv()), ClientIncoming::Closed(_)));
    assert_eq!(refusal(RelayClient::join(addr, &code)), Refusal::NoRoom);
}

#[test]
fn wrong_code_and_full_room_are_refused() {
    let addr = start(Config { max_clients_per_room: 2, ..Config::default() });
    let (code, host) = RelayClient::host(addr).unwrap();
    let other = if code.starts_with('Z') { "YYYYYY" } else { "ZZZZZZ" };
    assert_eq!(refusal(RelayClient::join(addr, other)), Refusal::NoRoom);
    assert_eq!(refusal(RelayClient::join(addr, "nope")), Refusal::BadRequest);

    let (_id1, _c1) = RelayClient::join(addr, &code).unwrap();
    let (_id2, c2) = RelayClient::join(addr, &code).unwrap();
    assert_eq!(refusal(RelayClient::join(addr, &code)), Refusal::Full);

    // A freed seat is reused.
    drop(c2);
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Joined(1));
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Joined(2));
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Left(2));
    let (id, _c2) = RelayClient::join(addr, &code).unwrap();
    assert_eq!(id, 2);
}

#[test]
fn room_limit_refuses_hosts() {
    let addr = start(Config { max_rooms: 1, ..Config::default() });
    let _room = RelayClient::host(addr).unwrap();
    assert!(matches!(RelayClient::host(addr), Err(RelayError::Refused(Refusal::Full))));
}

#[test]
fn join_attempts_are_rate_limited() {
    let addr = start(Config::default());
    let (code, _host) = RelayClient::host(addr).unwrap();
    let other = if code.starts_with('Z') { "YYYYYY" } else { "ZZZZZZ" };
    for _ in 0..10 {
        assert_eq!(refusal(RelayClient::join(addr, other)), Refusal::NoRoom);
    }
    assert_eq!(refusal(RelayClient::join(addr, other)), Refusal::RateLimited);
    assert_eq!(refusal(RelayClient::join(addr, &code)), Refusal::RateLimited);
}

#[test]
fn keepalives_hold_idle_links_open() {
    // Shorter than the test's silence: only the links' 5 s keepalives keep them alive.
    let addr = start(Config { idle_timeout: Duration::from_secs(7), ..Config::default() });
    let (code, host) = RelayClient::host(addr).unwrap();
    let (id, client) = RelayClient::join(addr, &code).unwrap();
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Joined(id));
    thread::sleep(Duration::from_secs(9));
    assert_eq!(host.try_recv(), None);
    assert_eq!(client.try_recv(), None);
    client.send(b"still here").unwrap();
    let msg = wait(|| host.try_recv());
    assert_eq!(msg, HostIncoming::Data { from: id, bytes: b"still here".to_vec() });
}

/// Waits for the server to close `stream`, failing on a timeout.
fn assert_closed(mut stream: TcpStream) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    match stream.read(&mut [0; 1]) {
        Ok(0) => {}
        Err(e) if !matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
        other => panic!("expected the relay to close the connection, got {other:?}"),
    }
}

#[test]
fn silent_and_oversized_connections_are_dropped() {
    let addr = start(Config { handshake_timeout: Duration::from_millis(200), ..Config::default() });
    assert_closed(TcpStream::connect(addr).unwrap());

    let mut garbage = TcpStream::connect(addr).unwrap();
    write_frame(&mut garbage, b"GET / HTTP/1.1").unwrap();
    assert_eq!(read_frame(&mut garbage).unwrap(), b"ERR bad_request");

    let mut host = TcpStream::connect(addr).unwrap();
    write_frame(&mut host, b"RRLY1 HOST").unwrap();
    assert!(read_frame(&mut host).unwrap().starts_with(b"OK "));
    host.write_all(&(MAX_FRAME as u32 + 1).to_be_bytes()).unwrap();
    assert_closed(host);
}
