//! Sessions: solo, and over a real relay on 127.0.0.1.

use raft_net::{new_token, Session, Status};
use raft_relay::{ClientIncoming, Config, RelayClient, Server};
use raft_sim::{Event, FlatGround, PlayerId, Request, World, V3};
use std::thread;
use std::time::{Duration, Instant};

const DT: f32 = 1.0 / 60.0;

fn relay() -> String {
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    };
    let server = Server::bind(config).unwrap();
    let addr = server.local_addr().unwrap();
    thread::spawn(move || -> () { server.run() });
    addr.to_string()
}

/// Ticks every session (about 2x real time, well inside the relay's byte budget) and logs
/// their events until `done` holds.
fn until(
    s: &mut [&mut Session],
    log: &mut [Vec<Event>],
    what: &str,
    done: impl Fn(&[&mut Session], &[Vec<Event>]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(s, log) {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        for (s, log) in s.iter_mut().zip(log.iter_mut()) {
            log.extend(s.update(DT, &FlatGround::WATER));
        }
        thread::sleep(Duration::from_millis(8));
    }
}

fn denied(log: &[Event], pid: PlayerId) -> bool {
    log.iter()
        .any(|e| matches!(e, Event::Denied { player, .. } if *player == pid))
}

fn pos_of(s: &Session, pid: PlayerId) -> V3 {
    s.world().unwrap().player(pid).unwrap().pos
}

#[test]
fn host_and_two_clients_play_together() {
    let addr = relay();
    let mut host = Session::host(&addr, World::new(1), "Hana");
    assert!(host.is_host());
    assert_eq!(host.status(), Status::Connecting);
    assert!(
        host.world().is_some() && host.me().is_some(),
        "the host plays while connecting"
    );
    let mut log = vec![Vec::new(); 3];
    until(&mut [&mut host], &mut log, "room code", |s, _| {
        s[0].status() == Status::Playing
    });
    let code = host.room_code().unwrap().to_string();

    let (ta, tb) = (new_token(), new_token());
    let mut a = Session::join(&addr, &code.to_lowercase(), "Ana", &ta);
    let mut b = Session::join(&addr, &code, "Bo", &tb);
    assert!(!a.is_host() && a.world().is_none() && a.me().is_none());
    assert_eq!(a.status(), Status::Connecting);
    until(
        &mut [&mut host, &mut a, &mut b],
        &mut log,
        "welcomes",
        |s, _| s[1].status() == Status::Playing && s[2].status() == Status::Playing,
    );
    let (h, ma, mb) = (host.me().unwrap(), a.me().unwrap(), b.me().unwrap());
    assert!(h != ma && ma != mb && h != mb);
    assert_eq!(a.room_code(), Some(code.as_str()));
    until(
        &mut [&mut host, &mut a, &mut b],
        &mut log,
        "rosters",
        |s, _| {
            s.iter().all(|s| s.peer_count() == 2)
                && s[2]
                    .world()
                    .unwrap()
                    .player(ma)
                    .is_some_and(|p| p.name == "Ana")
        },
    );

    // Client-authoritative movement reaches the host and the other client.
    let to = V3::new(5.0, 0.0, 30.0);
    a.request(Request::PlayerTransform {
        pos: to,
        yaw: 1.0,
        in_water: true,
        sprinting: false,
    });
    assert_eq!(pos_of(&a, ma), to, "the own mirror moves at once");
    until(
        &mut [&mut host, &mut a, &mut b],
        &mut log,
        "pose",
        |s, _| pos_of(s[0], ma).dist(to) < 0.01 && pos_of(s[2], ma).dist(to) < 0.02,
    );

    // Far from the logbook with no coins: denied, and only Ana hears about it.
    a.request(Request::SaveWorld);
    until(
        &mut [&mut host, &mut a, &mut b],
        &mut log,
        "denial",
        |_, log| denied(&log[1], ma),
    );
    assert!(denied(&log[0], ma), "the host sees every event");
    assert!(log[2].iter().all(|e| !matches!(e, Event::Denied { .. })));

    // Slow sections keep the mirrors' clocks close to the host's.
    until(
        &mut [&mut host, &mut a, &mut b],
        &mut log,
        "clock",
        |s, _| s[1].world().unwrap().clock.elapsed > 1.0,
    );
    let lag = host.world().unwrap().clock.elapsed - a.world().unwrap().clock.elapsed;
    assert!((0.0..0.5).contains(&lag), "clock lag {lag}");

    // Reload: everyone is re-joined by name and resynced.
    let mut save = World::new(9);
    save.economy.save_count = 42;
    host.replace_world(World::from_json(&save.to_json()).unwrap());
    until(
        &mut [&mut host, &mut a, &mut b],
        &mut log,
        "resync",
        |s, _| {
            s[1..]
                .iter()
                .all(|c| c.world().unwrap().economy.save_count == 42)
        },
    );
    let w = host.world().unwrap();
    assert_eq!((w.seed, w.connected().count()), (9, 3));
    let (ma, mb) = (a.me().unwrap(), b.me().unwrap());
    assert_eq!(w.player(ma).unwrap().name, "Ana");
    assert_eq!(w.player(mb).unwrap().name, "Bo");
    assert_eq!(a.world().unwrap().seed, 9);

    // Bo leaves: the host and Ana see him disconnected.
    drop(b);
    until(&mut [&mut host, &mut a], &mut log, "leave", |s, _| {
        s[..2]
            .iter()
            .all(|s| s.world().unwrap().player(mb).is_some_and(|p| !p.connected))
    });
    assert_eq!(host.peer_count(), 1);

    // Only Bo's token gets Bo's entry back.
    let mut fake = Session::join(&addr, &code, "Bo", &new_token());
    until(
        &mut [&mut host, &mut a, &mut fake],
        &mut log,
        "impostor refused",
        |s, _| s[2].status() == Status::Closed("name taken".into()),
    );
    let mut b = Session::join(&addr, &code, "Bo", &tb);
    until(
        &mut [&mut host, &mut a, &mut b],
        &mut log,
        "Bo back",
        |s, _| s[2].status() == Status::Playing,
    );
    assert_eq!(b.me(), Some(mb));
    assert_eq!(host.peer_count(), 2);

    drop(host);
    until(&mut [&mut a], &mut log, "host left", |s, _| {
        s[0].status() == Status::Closed("Host left".into())
    });
}

#[test]
fn bad_clients_are_ignored_or_rejected() {
    let addr = relay();
    let mut host = Session::host(&addr, World::new(1), "Hana");
    let mut log = vec![Vec::new(); 3];
    until(&mut [&mut host], &mut log, "room code", |s, _| {
        s[0].status() == Status::Playing
    });
    let code = host.room_code().unwrap().to_string();

    let (_, raw) = RelayClient::join(addr.as_str(), &code).unwrap();
    raw.send(b"not json").unwrap();
    raw.send(br#"{"Req":"Eat"}"#).unwrap(); // before Hello
    raw.send(br#"{"Hello":{"version":999,"name":"Old"}}"#)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut replies = Vec::new();
    while replies.len() < 2 {
        assert!(Instant::now() < deadline, "no reply to a wrong version");
        host.update(DT, &FlatGround::WATER);
        if let Some(ClientIncoming::Data(bytes)) = raw.try_recv() {
            replies.push(String::from_utf8(bytes).unwrap());
        }
        thread::sleep(Duration::from_millis(8));
    }
    assert!(replies[0].contains("Challenge"), "{replies:?}");
    assert!(
        replies[1].contains("Reject") && replies[1].contains("Version mismatch"),
        "{replies:?}"
    );
    assert_eq!(host.peer_count(), 0);
    assert_eq!(host.world().unwrap().connected().count(), 1);

    let mut bad = Session::join(&addr, &code, "Cy", "not hex");
    assert_eq!(bad.status(), Status::Closed("bad token".into()));
    assert!(bad.update(DT, &FlatGround::WATER).is_empty());

    let mut taken = Session::join(&addr, &code, "Hana", &new_token());
    let other = if code.starts_with('Z') {
        "YYYYYY"
    } else {
        "ZZZZZZ"
    };
    let mut lost = Session::join(&addr, other, "Ana", &new_token());
    until(
        &mut [&mut host, &mut taken, &mut lost],
        &mut log,
        "refusals",
        |s, _| {
            matches!(s[2].status(), Status::Closed(_))
                && s[1].status() == Status::Closed("name taken".into())
        },
    );
}

/// Hosts a fresh world with seed `seed`, returning the session and its room code.
fn hosted(addr: &str, seed: u64, log: &mut [Vec<Event>]) -> (Session, String) {
    let mut host = Session::host(addr, World::new(seed), "Hana");
    until(&mut [&mut host], log, "room code", |s, _| {
        s[0].status() == Status::Playing
    });
    let code = host.room_code().unwrap().to_string();
    (host, code)
}

#[test]
fn loading_another_worlds_save_challenges_again() {
    let addr = relay();
    let mut log = vec![Vec::new(); 2];
    let secret = new_token();
    let (mut first, code) = hosted(&addr, 1, &mut log);
    let mut ana = Session::join(&addr, &code, "Ana", &secret);
    until(&mut [&mut first, &mut ana], &mut log, "welcome", |s, _| {
        s[1].status() == Status::Playing
    });
    let save = first.save_json().unwrap();
    drop((first, ana));

    // Ana plays a new world on another host, which then loads the first world's save.
    let (mut host, code) = hosted(&addr, 2, &mut log);
    let mut ana = Session::join(&addr, &code, "Ana", &secret);
    until(&mut [&mut host, &mut ana], &mut log, "welcome", |s, _| {
        s[1].status() == Status::Playing
    });
    host.replace_world(World::load(&save).unwrap());
    host.restore_owners(&save);
    until(&mut [&mut host, &mut ana], &mut log, "answer", |s, _| {
        s[0].peer_count() == 1 && s[1].world().unwrap().seed == 1
    });
    assert_eq!(ana.status(), Status::Playing);
}

#[test]
fn solo_runs_without_network() {
    assert!(Session::solo(World::new(1), "  ").is_err());
    let mut s = Session::solo(World::new(1), "Solo").unwrap();
    let me = s.me().unwrap();
    assert!(s.is_host());
    assert_eq!(
        (s.status(), s.room_code(), s.peer_count()),
        (Status::Playing, None, 0)
    );

    s.request(Request::SaveWorld);
    let ev = s.update(DT, &FlatGround::WATER);
    assert!(denied(&ev, me), "{ev:?}");
    let t = s.world().unwrap().clock.elapsed;
    s.update(DT, &FlatGround::WATER);
    assert!(s.world().unwrap().clock.elapsed > t);

    let saved = World::load(&s.save_json().unwrap()).unwrap();
    s.replace_world(saved);
    assert_eq!(
        s.world().unwrap().player(s.me().unwrap()).unwrap().name,
        "Solo"
    );
    assert_eq!(s.world().unwrap().connected().count(), 1);
}
