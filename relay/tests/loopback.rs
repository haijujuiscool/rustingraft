//! Real server and clients over 127.0.0.1.

use raft_relay::{
    read_frame, write_frame, ClientIncoming, ClientLink, Config, HostIncoming, Refusal,
    RelayClient, RelayError, Server, MAX_FRAME,
};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
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

#[test]
fn trickled_handshakes_hit_the_deadline() {
    let addr = start(Config { handshake_timeout: Duration::from_millis(300), ..Config::default() });
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    let started = Instant::now();
    // A 100-byte handshake at one byte per 100 ms: every read makes progress, the whole never
    // finishes in time.
    for byte in [0u8, 0, 0, 100].into_iter().chain(std::iter::repeat(b'x')) {
        assert!(started.elapsed() < Duration::from_millis(1500), "trickling peer never dropped");
        if stream.write_all(&[byte]).is_err() {
            break;
        }
        match stream.read(&mut [0; 1]) {
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            _ => break, // closed or reset
        }
    }
}

#[test]
fn a_client_that_stops_reading_cannot_stall_its_room() {
    let addr = start(Config::default());
    let (code, host) = RelayClient::host(addr).unwrap();
    let (id, fast) = RelayClient::join(addr, &code).unwrap();
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Joined(id));
    let mut stuck = TcpStream::connect(addr).unwrap();
    write_frame(&mut stuck, format!("RRLY1 JOIN {code}").as_bytes()).unwrap();
    assert_eq!(read_frame(&mut stuck).unwrap(), b"OK 2");
    assert_eq!(wait(|| host.try_recv()), HostIncoming::Joined(2));
    keep_alive(&stuck);

    // `stuck` stops reading but, like a game whose main thread hangs, keeps sending keepalives. Broadcast within the host's allowance (the burst, then under
    // 256 KiB/s): the reading client gets every frame promptly, and `stuck` is dropped once
    // the relay has queued MAX_QUEUED bytes for it.
    let chunk = vec![7u8; 60 * 1024];
    let mut sent = 0;
    for i in 0.. {
        assert!(i < 40, "the stuck client was never dropped");
        if i >= 8 {
            thread::sleep(Duration::from_millis(250));
        }
        host.send_to(0, &chunk).unwrap();
        sent += 4 + chunk.len();
        let started = Instant::now();
        loop {
            match fast.try_recv() {
                Some(ClientIncoming::Data(d)) if d.len() == chunk.len() => break,
                Some(other) => panic!("unexpected {other:?}"),
                None => {
                    assert!(started.elapsed() < Duration::from_secs(1), "frame {i} stalled");
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        if let Some(msg) = host.try_recv() {
            assert_eq!(msg, HostIncoming::Left(2));
            break;
        }
    }

    // What the kernel already took still arrives, then the connection ends.
    stuck.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut received = 0;
    let mut buf = [0; 65_536];
    loop {
        match stuck.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => received += n,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                panic!("stuck client still open after {received} bytes")
            }
            Err(_) => break, // reset
        }
    }
    assert!(received < sent, "got {received} of {sent} bytes");

    fast.send(b"room still works").unwrap();
    let msg = wait(|| host.try_recv());
    assert_eq!(msg, HostIncoming::Data { from: id, bytes: b"room still works".to_vec() });
}

#[test]
fn room_opens_are_rate_limited() {
    let addr = start(Config { max_conns_per_ip: 100, ..Config::default() });
    let rooms: Vec<_> = (0..10).map(|_| RelayClient::host(addr).unwrap()).collect();
    assert!(matches!(RelayClient::host(addr), Err(RelayError::Refused(Refusal::RateLimited))));
    assert!(RelayClient::join(addr, &rooms[0].0).is_ok(), "joins have their own budget");
}

#[test]
fn finished_sessions_free_their_connection_slots() {
    let addr = start(Config { max_conns: 2, ..Config::default() });
    for round in 0..3u8 {
        // Refused at the connection cap, which counts no attempt; slots free as soon as both
        // threads of the previous round's connections end.
        let (code, host) = wait(|| RelayClient::host(addr).ok());
        let (id, client) = wait(|| RelayClient::join(addr, &code).ok());
        assert!(RelayClient::host(addr).is_err(), "round {round}: cap exceeded");
        client.send(&[round]).unwrap();
        assert_eq!(wait(|| host.try_recv()), HostIncoming::Joined(id));
        assert_eq!(wait(|| host.try_recv()), HostIncoming::Data { from: id, bytes: vec![round] });
    }
}

/// Sends a keepalive every 100 ms until the connection fails.
fn keep_alive(stream: &TcpStream) {
    let mut stream = stream.try_clone().unwrap();
    thread::spawn(move || {
        while write_frame(&mut stream, &[]).is_ok() {
            thread::sleep(Duration::from_millis(100));
        }
    });
}

/// Accepts one connection, answers its handshake with `reply` and hands the socket over.
fn fake_relay(reply: &'static [u8]) -> (SocketAddr, mpsc::Receiver<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_frame(&mut stream).unwrap();
        write_frame(&mut stream, reply).unwrap();
        let _ = tx.send(stream);
    });
    (addr, rx)
}

#[test]
fn a_relay_that_stops_reading_closes_the_link() {
    let (addr, relay) = fake_relay(b"OK 1");
    let (_, link) = RelayClient::join(addr, "AB23CD").unwrap();
    let relay = relay.recv().unwrap();
    keep_alive(&relay); // and never reads
    let chunk = vec![1u8; raft_relay::MAX_DATA];
    let mut sent = 0;
    let err = loop {
        match link.send(&chunk) {
            Ok(()) => sent += 1,
            Err(e) => break e,
        }
        assert!(sent < 2000, "the send queue never filled");
    };
    assert_eq!(err.kind(), ErrorKind::WouldBlock);
    assert!(matches!(wait(|| link.try_recv()), ClientIncoming::Closed(_)));
}

#[test]
fn an_unpolled_link_pushes_back_on_the_relay() {
    let (addr, relay) = fake_relay(b"OK 1");
    let (_, link) = RelayClient::join(addr, "AB23CD").unwrap();
    let mut relay = relay.recv().unwrap();
    relay.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
    let chunk = vec![2u8; MAX_FRAME];
    let mut written = 0;
    while write_frame(&mut relay, &chunk).is_ok() {
        written += 1;
        assert!(written < 2000, "the link kept reading with nobody polling it");
    }
    assert!(written >= raft_relay::client::LINK_QUEUE, "only {written} frames buffered");
    assert_eq!(wait(|| link.try_recv()), ClientIncoming::Data(chunk));
}

#[test]
fn a_relay_that_never_answers_cannot_hang_a_join() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        // Keepalives only, each well inside a per-read timeout.
        keep_alive(&stream);
        thread::sleep(Duration::from_secs(10));
    });
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(RelayClient::join(addr, "AB23CD").map(|(id, _)| id));
    });
    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Err(RelayError::Io(_))) => {}
        other => panic!("expected the handshake to time out, got {other:?}"),
    }
}
