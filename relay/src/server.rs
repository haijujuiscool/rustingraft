//! The relay server: a reader thread per connection and, once it is in a room, a writer
//! thread; every resource bounded.
//!
//! Lock order is state lock, then a peer's outbox lock. Nothing writes to a socket while
//! holding a lock.

use crate::code::{generate_code, is_valid_code};
use crate::frame::{encode_frame, read_frame, write_frame, Deadline};
use crate::limit::{AttemptLimiter, IpLimiter, TokenBucket};
use crate::proto::{
    err_reply, ok_reply, Control, Handshake, Refusal, BROADCAST, CONTROL, DEFAULT_PORT,
    HANDSHAKE_TIMEOUT, IDLE_TIMEOUT, KEEPALIVE_INTERVAL, MAX_CLIENT_ID,
};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// A write that makes no progress for this long kills the connection.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Bytes queued to one peer and not yet handed to the kernel. A peer this far behind is
/// dropped, so a slow reader can neither stall its room nor grow memory: 512 connections x
/// 256 KiB stays well inside the unit's `MemoryMax`.
pub const MAX_QUEUED: usize = 256 * 1024;
const THREAD_STACK: usize = 256 * 1024;

/// Never panics, unlike `eprintln!` when stderr is gone: a panicking session would leak its
/// room.
macro_rules! log {
    ($($arg:tt)*) => {{
        let _ = writeln!(io::stderr(), $($arg)*);
    }};
}

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
    joins: AttemptLimiter,
    hosts: AttemptLimiter,
    rooms: HashMap<String, Room>,
    next_serial: u64,
}

struct Room {
    /// Tells a room apart from a later one that happens to reuse its code.
    serial: u64,
    host: Arc<Peer>,
    clients: BTreeMap<u8, Arc<Peer>>,
}

/// A whole encoded frame, shared by every recipient of a broadcast.
type Frame = Arc<[u8]>;

/// One connection past its handshake. Sending only queues; the peer's own writer thread
/// puts frames on the wire, so no thread ever blocks on someone else's socket.
struct Peer {
    stream: TcpStream,
    outbox: Mutex<Option<Sender<Frame>>>,
    queued: AtomicUsize,
}

impl Peer {
    fn new(stream: TcpStream) -> (Arc<Self>, Receiver<Frame>) {
        let (tx, rx) = mpsc::channel();
        let peer = Self { stream, outbox: Mutex::new(Some(tx)), queued: AtomicUsize::new(0) };
        (Arc::new(peer), rx)
    }

    /// Queues `frame`, or closes the peer if that would put it over [`MAX_QUEUED`]: a lost
    /// frame would desync the game, so falling behind ends the connection.
    fn send(&self, frame: Frame) {
        let queued = self.queued.fetch_add(frame.len(), Ordering::Relaxed) + frame.len();
        let sent = queued <= MAX_QUEUED
            && lock(&self.outbox).as_ref().is_some_and(|tx| tx.send(frame).is_ok());
        if !sent {
            self.close();
        }
    }

    /// Stops the writer and ends this peer's session: at once on Linux, where shutdown wakes
    /// the blocked read; on Windows, which keeps delivering data after a shutdown, when the
    /// session sees [`Peer::is_closed`] at the peer's next frame or idle timeout.
    fn close(&self) {
        lock(&self.outbox).take();
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    fn is_closed(&self) -> bool {
        lock(&self.outbox).is_none()
    }
}

/// Writes queued frames, and a keepalive after [`KEEPALIVE_INTERVAL`] of silence, until the
/// peer is closed or a write fails.
fn write_loop(peer: &Peer, frames: Receiver<Frame>) {
    loop {
        let written = match frames.recv_timeout(KEEPALIVE_INTERVAL) {
            Ok(frame) => {
                let written = (&peer.stream).write_all(&frame);
                peer.queued.fetch_sub(frame.len(), Ordering::Relaxed);
                written
            }
            Err(RecvTimeoutError::Timeout) => (&peer.stream).write_all(&[0; 4]),
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if written.is_err() {
            break;
        }
    }
    peer.close();
}

/// Starts `peer`'s writer thread, which holds the connection's slot until it ends. If the
/// thread cannot start, the peer is closed and its session ends through the usual teardown.
fn start_writer(peer: &Arc<Peer>, frames: Receiver<Frame>, guard: Arc<ConnGuard>) {
    let writer = Arc::clone(peer);
    let spawned = thread::Builder::new().name("relay-write".into()).stack_size(THREAD_STACK).spawn(
        move || {
            let _guard = guard;
            write_loop(&writer, frames);
        },
    );
    if spawned.is_err() {
        peer.close();
    }
}

impl Server {
    pub fn bind(config: Config) -> io::Result<Self> {
        let listener = TcpListener::bind(config.bind)?;
        let now = Instant::now();
        let state = State {
            conns: 0,
            ips: IpLimiter::new(config.max_conns_per_ip),
            joins: AttemptLimiter::new(now),
            hosts: AttemptLimiter::new(now),
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
                // Not logged per connection: a connect flood would flood the journal.
                Ok((stream, addr)) => match admit(&self.shared, addr.ip()) {
                    Some(guard) => {
                        let shared = Arc::clone(&self.shared);
                        let spawned = thread::Builder::new()
                            .name("relay-conn".into())
                            .stack_size(THREAD_STACK)
                            .spawn(move || handle(&shared, stream, addr.ip(), guard));
                        if let Err(e) = spawned {
                            log!("cannot spawn a connection thread: {e}");
                        }
                    }
                    None => refused += 1,
                },
                Err(e) => {
                    // Usually out of file descriptors: back off instead of spinning.
                    log!("accept failed: {e}");
                    thread::sleep(Duration::from_millis(100));
                }
            }
            if refused > 0 && last_report.elapsed() >= Duration::from_secs(10) {
                log!("refused {refused} connections over the connection limits");
                refused = 0;
                last_report = Instant::now();
            }
        }
    }
}

/// Releases a connection's slot once its reader and writer threads have both ended, so
/// there are never more than `2 * max_conns` connection threads.
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

fn admit(shared: &Arc<Shared>, ip: IpAddr) -> Option<ConnGuard> {
    let mut st = lock(&shared.state);
    if st.conns >= shared.config.max_conns || !st.ips.connect(ip) {
        return None;
    }
    st.conns += 1;
    Some(ConnGuard { shared: Arc::clone(shared), ip })
}

fn handle(shared: &Shared, stream: TcpStream, ip: IpAddr, guard: ConnGuard) {
    let setup =
        stream.set_nodelay(true).and_then(|()| stream.set_write_timeout(Some(WRITE_TIMEOUT)));
    if setup.is_err() {
        return;
    }
    let mut hello = Deadline::new(&stream, shared.config.handshake_timeout);
    let Ok(hello) = read_frame(&mut hello) else { return };
    let guard = Arc::new(guard);
    match Handshake::parse(&hello) {
        Some(Handshake::Host) => host_session(shared, stream, ip, guard),
        Some(Handshake::Join(code)) => client_session(shared, stream, ip, &code, guard),
        None => refuse(&stream, Refusal::BadRequest),
    }
}

/// Refusals go out before the peer has a writer thread, straight on the socket.
fn refuse(stream: &TcpStream, refusal: Refusal) {
    let _ = write_frame(&mut &*stream, &err_reply(refusal));
}

/// An encoded frame; `None` if it would be over the frame limit.
fn frame(prefix: &[u8], payload: &[u8]) -> Option<Frame> {
    encode_frame(prefix, payload).ok().map(Frame::from)
}

fn host_session(shared: &Shared, stream: TcpStream, ip: IpAddr, guard: Arc<ConnGuard>) {
    let (peer, frames) = Peer::new(stream);
    let opened = open_room(&mut lock(&shared.state), &shared.config, ip, &peer);
    let (code, rooms) = match opened {
        Ok(room) => room,
        Err(refusal) => {
            refuse(&peer.stream, refusal);
            if refusal != Refusal::RateLimited {
                log!("host {ip} refused: room limit reached");
            }
            return;
        }
    };
    start_writer(&peer, frames, guard);
    log!("room {code} opened by {ip} (rooms {rooms})");

    if peer.stream.set_read_timeout(Some(shared.config.idle_timeout)).is_ok() {
        let mut bucket = TokenBucket::new(Instant::now());
        while let Ok(data) = read_frame(&mut &peer.stream) {
            if peer.is_closed() {
                break;
            }
            if !bucket.take(data.len() + 4, Instant::now()) {
                log!("room {code}: host {ip} over the bandwidth limit, dropped");
                break;
            }
            let Some((&target, payload)) = data.split_first() else { continue };
            if payload.is_empty() {
                continue;
            }
            let clients = recipients(shared, &code, target);
            if clients.is_empty() {
                continue;
            }
            let Some(out) = frame(&[], payload) else { break };
            for client in clients {
                client.send(Arc::clone(&out));
            }
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
    log!("room {code} closed, {} clients dropped (rooms {rooms})", clients.len());
}

/// Registers a room for `host` and queues its `OK <code>` before any client can find the
/// room, so no `JOINED` can overtake it. Returns the code and the room count.
fn open_room(
    st: &mut State,
    config: &Config,
    ip: IpAddr,
    host: &Arc<Peer>,
) -> Result<(String, usize), Refusal> {
    if !st.hosts.attempt(ip, Instant::now()) {
        return Err(Refusal::RateLimited);
    }
    if st.rooms.len() >= config.max_rooms {
        return Err(Refusal::Full);
    }
    let code = loop {
        let code = generate_code();
        if !st.rooms.contains_key(&code) {
            break code;
        }
    };
    let ok = frame(&[], &ok_reply(&code)).ok_or(Refusal::BadRequest)?;
    st.next_serial += 1;
    let room = Room { serial: st.next_serial, host: Arc::clone(host), clients: BTreeMap::new() };
    st.rooms.insert(code.clone(), room);
    host.send(ok);
    Ok((code, st.rooms.len()))
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

fn client_session(
    shared: &Shared,
    stream: TcpStream,
    ip: IpAddr,
    code: &str,
    guard: Arc<ConnGuard>,
) {
    let (peer, frames) = Peer::new(stream);
    let seat = take_seat(&mut lock(&shared.state), &shared.config, ip, code, &peer);
    let Seat { id, serial, host, clients } = match seat {
        Ok(seat) => seat,
        Err(refusal) => {
            refuse(&peer.stream, refusal);
            // Rate-limited attempts are not logged: that is the flood.
            if refusal != Refusal::RateLimited {
                log!("join from {ip} refused: {refusal}");
            }
            return;
        }
    };
    start_writer(&peer, frames, guard);
    if let Some(joined) = frame(&[CONTROL], &Control::Joined(id).encode()) {
        host.send(joined);
    }
    log!("room {code}: client {id} joined from {ip} (clients {clients})");

    if peer.stream.set_read_timeout(Some(shared.config.idle_timeout)).is_ok() {
        let mut bucket = TokenBucket::new(Instant::now());
        while let Ok(data) = read_frame(&mut &peer.stream) {
            if peer.is_closed() {
                break;
            }
            if !bucket.take(data.len() + 4, Instant::now()) {
                log!("room {code}: client {id} over the bandwidth limit, dropped");
                break;
            }
            if data.is_empty() {
                continue;
            }
            // Over the limit means no room for the id prefix.
            let Some(out) = frame(&[id], &data) else { break };
            host.send(out);
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
        if let Some(left) = frame(&[CONTROL], &Control::Left(id).encode()) {
            host.send(left);
        }
        log!("room {code}: client {id} left (clients {clients})");
    }
    peer.close();
}

/// Seats `peer` in room `code` and queues its `OK <id>` before the host can address it.
fn take_seat(
    st: &mut State,
    config: &Config,
    ip: IpAddr,
    code: &str,
    peer: &Arc<Peer>,
) -> Result<Seat, Refusal> {
    if !st.joins.attempt(ip, Instant::now()) {
        return Err(Refusal::RateLimited);
    }
    if !is_valid_code(code) {
        return Err(Refusal::BadRequest);
    }
    let room = st.rooms.get_mut(code).ok_or(Refusal::NoRoom)?;
    let max = config.max_clients_per_room.min(MAX_CLIENT_ID);
    let id = (1..=max).find(|id| !room.clients.contains_key(id)).ok_or(Refusal::Full)?;
    let ok = frame(&[], &ok_reply(&id.to_string())).ok_or(Refusal::BadRequest)?;
    room.clients.insert(id, Arc::clone(peer));
    peer.send(ok);
    Ok(Seat { id, serial: room.serial, host: Arc::clone(&room.host), clients: room.clients.len() })
}

/// Keeps serving even if some thread panicked while holding a lock.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
