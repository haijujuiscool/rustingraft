//! The relay server: one thread per connection, every resource bounded.
//!
//! Lock order is always peer write lock, then state lock. Nothing writes to a socket while
//! holding the state lock.

use crate::code::{generate_code, is_valid_code};
use crate::frame::{encode_frame, read_frame, MAX_DATA};
use crate::limit::{IpLimiter, TokenBucket};
use crate::proto::{
    err_reply, ok_reply, Control, Handshake, Refusal, BROADCAST, CONTROL, DEFAULT_PORT,
    HANDSHAKE_TIMEOUT, IDLE_TIMEOUT, KEEPALIVE_INTERVAL, MAX_CLIENT_ID,
};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// A write that makes no progress for this long kills the connection.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const THREAD_STACK: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub max_rooms: usize,
    /// Clamped to [`MAX_CLIENT_ID`].
    pub max_clients_per_room: u8,
    pub max_conns: usize,
    pub max_conns_per_ip: usize,
    pub handshake_timeout: Duration,
    pub idle_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, DEFAULT_PORT)),
            max_rooms: 64,
            max_clients_per_room: MAX_CLIENT_ID,
            max_conns: 512,
            max_conns_per_ip: 8,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            idle_timeout: IDLE_TIMEOUT,
        }
    }
}

pub struct Server {
    listener: TcpListener,
    shared: Arc<Shared>,
}

struct Shared {
    config: Config,
    state: Mutex<State>,
}

struct State {
    conns: usize,
    ips: IpLimiter,
    rooms: HashMap<String, Room>,
    next_serial: u64,
}

struct Room {
    /// Tells a room apart from a later one that happens to reuse its code.
    serial: u64,
    host: Arc<Peer>,
    clients: BTreeMap<u8, Arc<Peer>>,
}

/// One connection. The mutex serializes whole frames and remembers the last write, for
/// keepalives.
struct Peer {
    stream: TcpStream,
    last_sent: Mutex<Instant>,
}

impl Peer {
    fn send(&self, prefix: &[u8], payload: &[u8]) {
        let mut last_sent = lock(&self.last_sent);
        self.send_locked(&mut last_sent, prefix, payload);
    }

    /// A failed or timed-out write may leave half a frame on the wire, so it closes the
    /// connection; its own thread then sees the error and cleans up.
    fn send_locked(&self, last_sent: &mut Instant, prefix: &[u8], payload: &[u8]) {
        match encode_frame(prefix, payload).and_then(|frame| (&self.stream).write_all(&frame)) {
            Ok(()) => *last_sent = Instant::now(),
            Err(_) => self.close(),
        }
    }

    fn keepalive_if_due(&self) {
        let mut last_sent = lock(&self.last_sent);
        if last_sent.elapsed() >= KEEPALIVE_INTERVAL {
            self.send_locked(&mut last_sent, &[], &[]);
        }
    }

    fn close(&self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl Server {
    pub fn bind(config: Config) -> io::Result<Self> {
        let listener = TcpListener::bind(config.bind)?;
        let state = State {
            conns: 0,
            ips: IpLimiter::new(config.max_conns_per_ip),
            rooms: HashMap::new(),
            next_serial: 0,
        };
        let shared = Arc::new(Shared { config, state: Mutex::new(state) });
        Ok(Self { listener, shared })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accepts connections forever.
    pub fn run(self) -> ! {
        let mut refused = 0u64;
        let mut last_report = Instant::now();
        loop {
            match self.listener.accept() {
                Ok((stream, addr)) => {
                    let ip = addr.ip();
                    match admit(&self.shared, ip) {
                        Some((guard, conns)) => {
                            eprintln!("connect {ip} (conns {conns})");
                            let shared = Arc::clone(&self.shared);
                            let spawned = thread::Builder::new()
                                .name("relay-conn".into())
                                .stack_size(THREAD_STACK)
                                .spawn(move || {
                                    let _guard = guard;
                                    handle(&shared, stream, ip);
                                });
                            if let Err(e) = spawned {
                                eprintln!("cannot spawn a connection thread: {e}");
                            }
                        }
                        None => refused += 1,
                    }
                }
                Err(e) => {
                    // Usually out of file descriptors: back off instead of spinning.
                    eprintln!("accept failed: {e}");
                    thread::sleep(Duration::from_millis(100));
                }
            }
            if refused > 0 && last_report.elapsed() >= Duration::from_secs(10) {
                eprintln!("refused {refused} connections over the connection limits");
                refused = 0;
                last_report = Instant::now();
            }
        }
    }
}

/// Releases a connection's slot when its thread ends, however it ends.
struct ConnGuard {
    shared: Arc<Shared>,
    ip: IpAddr,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let mut st = lock(&self.shared.state);
        st.conns -= 1;
        st.ips.disconnect(self.ip);
    }
}

fn admit(shared: &Arc<Shared>, ip: IpAddr) -> Option<(ConnGuard, usize)> {
    let mut st = lock(&shared.state);
    if st.conns >= shared.config.max_conns || !st.ips.connect(ip) {
        return None;
    }
    st.conns += 1;
    Some((ConnGuard { shared: Arc::clone(shared), ip }, st.conns))
}

fn handle(shared: &Shared, stream: TcpStream, ip: IpAddr) {
    let setup = stream
        .set_nodelay(true)
        .and_then(|()| stream.set_read_timeout(Some(shared.config.handshake_timeout)))
        .and_then(|()| stream.set_write_timeout(Some(WRITE_TIMEOUT)));
    if setup.is_err() {
        return;
    }
    let peer = Arc::new(Peer { stream, last_sent: Mutex::new(Instant::now()) });
    let Ok(hello) = read_frame(&mut &peer.stream) else { return };
    match Handshake::parse(&hello) {
        Some(Handshake::Host) => host_session(shared, peer, ip),
        Some(Handshake::Join(code)) => client_session(shared, peer, ip, &code),
        None => peer.send(&[], &err_reply(Refusal::BadRequest)),
    }
}

fn host_session(shared: &Shared, peer: Arc<Peer>, ip: IpAddr) {
    // Hold the host's write lock until `OK` is out, so no JOINED can overtake it.
    let mut last_sent = lock(&peer.last_sent);
    let mut st = lock(&shared.state);
    if st.rooms.len() >= shared.config.max_rooms {
        drop(st);
        peer.send_locked(&mut last_sent, &[], &err_reply(Refusal::Full));
        eprintln!("host {ip} refused: room limit reached");
        return;
    }
    let code = loop {
        let code = generate_code();
        if !st.rooms.contains_key(&code) {
            break code;
        }
    };
    st.next_serial += 1;
    let room = Room { serial: st.next_serial, host: Arc::clone(&peer), clients: BTreeMap::new() };
    st.rooms.insert(code.clone(), room);
    let rooms = st.rooms.len();
    drop(st);
    peer.send_locked(&mut last_sent, &[], &ok_reply(&code));
    drop(last_sent);
    eprintln!("room {code} opened by {ip} (rooms {rooms})");

    if peer.stream.set_read_timeout(Some(shared.config.idle_timeout)).is_ok() {
        let mut bucket = TokenBucket::new(Instant::now());
        while let Ok(frame) = read_frame(&mut &peer.stream) {
            if !bucket.take(frame.len() + 4, Instant::now()) {
                eprintln!("room {code}: host {ip} over the bandwidth limit, dropped");
                break;
            }
            if let Some((&target, payload)) = frame.split_first() {
                if !payload.is_empty() {
                    for client in recipients(shared, &code, target) {
                        client.send(&[], payload);
                    }
                }
            }
            peer.keepalive_if_due();
        }
    }

    let (clients, rooms) = {
        let mut st = lock(&shared.state);
        let clients = st.rooms.remove(&code).map(|room| room.clients).unwrap_or_default();
        (clients, st.rooms.len())
    };
    for client in clients.values() {
        client.close();
    }
    peer.close();
    eprintln!("room {code} closed, {} clients dropped (rooms {rooms})", clients.len());
}

fn recipients(shared: &Shared, code: &str, target: u8) -> Vec<Arc<Peer>> {
    let st = lock(&shared.state);
    let Some(room) = st.rooms.get(code) else { return Vec::new() };
    if target == BROADCAST {
        room.clients.values().cloned().collect()
    } else {
        room.clients.get(&target).cloned().into_iter().collect()
    }
}

struct Seat {
    id: u8,
    serial: u64,
    host: Arc<Peer>,
    clients: usize,
}

fn client_session(shared: &Shared, peer: Arc<Peer>, ip: IpAddr, code: &str) {
    // Hold this client's write lock until `OK` is out, so host data cannot overtake it.
    let mut last_sent = lock(&peer.last_sent);
    let seat = take_seat(&mut lock(&shared.state), &shared.config, ip, code, &peer);
    let Seat { id, serial, host, clients } = match seat {
        Ok(seat) => seat,
        Err(refusal) => {
            peer.send_locked(&mut last_sent, &[], &err_reply(refusal));
            // Rate-limited attempts are not logged: that is the flood.
            if refusal != Refusal::RateLimited {
                eprintln!("join from {ip} refused: {refusal}");
            }
            return;
        }
    };
    peer.send_locked(&mut last_sent, &[], &ok_reply(&id.to_string()));
    drop(last_sent);
    host.send(&[CONTROL], &Control::Joined(id).encode());
    eprintln!("room {code}: client {id} joined from {ip} (clients {clients})");

    if peer.stream.set_read_timeout(Some(shared.config.idle_timeout)).is_ok() {
        let mut bucket = TokenBucket::new(Instant::now());
        while let Ok(frame) = read_frame(&mut &peer.stream) {
            if !bucket.take(frame.len() + 4, Instant::now()) {
                eprintln!("room {code}: client {id} over the bandwidth limit, dropped");
                break;
            }
            if frame.len() > MAX_DATA {
                break; // no room for the id prefix
            }
            if !frame.is_empty() {
                host.send(&[id], &frame);
            }
            peer.keepalive_if_due();
        }
    }

    let remaining = {
        let mut st = lock(&shared.state);
        match st.rooms.get_mut(code) {
            Some(room) if room.serial == serial => {
                room.clients.remove(&id);
                Some(room.clients.len())
            }
            _ => None, // the room closed first
        }
    };
    if let Some(clients) = remaining {
        host.send(&[CONTROL], &Control::Left(id).encode());
        eprintln!("room {code}: client {id} left (clients {clients})");
    }
    peer.close();
}

fn take_seat(
    st: &mut State,
    config: &Config,
    ip: IpAddr,
    code: &str,
    peer: &Arc<Peer>,
) -> Result<Seat, Refusal> {
    if !st.ips.join_attempt(ip, Instant::now()) {
        return Err(Refusal::RateLimited);
    }
    if !is_valid_code(code) {
        return Err(Refusal::BadRequest);
    }
    let room = st.rooms.get_mut(code).ok_or(Refusal::NoRoom)?;
    let max = config.max_clients_per_room.min(MAX_CLIENT_ID);
    let id = (1..=max).find(|id| !room.clients.contains_key(id)).ok_or(Refusal::Full)?;
    room.clients.insert(id, Arc::clone(peer));
    Ok(Seat { id, serial: room.serial, host: Arc::clone(&room.host), clients: room.clients.len() })
}

/// Keeps serving even if some thread panicked while holding a lock.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
