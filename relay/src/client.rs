//! Game-side relay links. Each link owns a reader and a writer thread; the game polls
//! `try_recv` and queues sends without blocking. The writer sends a keepalive after 5 s of
//! silence; dropping a link flushes queued frames, then closes the connection.
//!
//! Both queues hold at most [`LINK_QUEUE`] frames. A full send queue closes the link (the
//! relay or the network cannot keep up); a full receive queue stops reading the socket until
//! the game polls, which pushes back on the relay instead of growing memory.

use crate::code::is_valid_code;
use crate::frame::{encode_frame, read_frame, write_frame, Deadline, MAX_DATA};
use crate::proto::{
    parse_reply, Control, Handshake, Refusal, CONTROL, HANDSHAKE_TIMEOUT, IDLE_TIMEOUT,
    KEEPALIVE_INTERVAL, MAX_CLIENT_ID,
};
use std::io::{self, ErrorKind, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::{error, fmt};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Frames each link queues per direction; at most 64 KiB each.
pub const LINK_QUEUE: usize = 256;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// What a host link receives. `Closed` is the last message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostIncoming {
    Joined(u8),
    Left(u8),
    Data { from: u8, bytes: Vec<u8> },
    Closed(String),
}

/// What a client link receives. `Closed` is the last message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientIncoming {
    Data(Vec<u8>),
    Closed(String),
}

#[derive(Debug)]
pub enum RelayError {
    Io(io::Error),
    /// The relay answered `ERR <reason>`.
    Refused(Refusal),
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "relay connection failed: {e}"),
            Self::Refused(reason) => write!(f, "relay refused: {reason}"),
        }
    }
}

impl error::Error for RelayError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Refused(_) => None,
        }
    }
}

impl From<io::Error> for RelayError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

pub struct RelayClient;

impl RelayClient {
    /// Opens a room and returns its code. Blocks up to ~10 s (connect + handshake).
    pub fn host<A: ToSocketAddrs>(addr: A) -> Result<(String, HostLink), RelayError> {
        let (stream, code) = handshake(addr, &Handshake::Host)?;
        if !is_valid_code(&code) {
            return Err(bad_reply().into());
        }
        Ok((code, HostLink(Link::spawn(stream, decode_host, HostIncoming::Closed)?)))
    }

    /// Joins a room (code is trimmed and upper-cased) and returns this client's id (1..=7).
    /// Blocks up to ~10 s (connect + handshake).
    pub fn join<A: ToSocketAddrs>(addr: A, code: &str) -> Result<(u8, ClientLink), RelayError> {
        let code = code.trim().to_ascii_uppercase();
        let (stream, id) = handshake(addr, &Handshake::Join(code))?;
        let id = id
            .parse::<u8>()
            .ok()
            .filter(|id| (1..=MAX_CLIENT_ID).contains(id))
            .ok_or_else(bad_reply)?;
        let decode = |frame| Some(ClientIncoming::Data(frame));
        Ok((id, ClientLink(Link::spawn(stream, decode, ClientIncoming::Closed)?)))
    }
}

pub struct HostLink(Link<HostIncoming>);

impl HostLink {
    pub fn try_recv(&self) -> Option<HostIncoming> {
        self.0.incoming.try_recv().ok()
    }

    /// Queues `bytes` for client `target`, or every client for target 0. Fails if `bytes`
    /// is over [`MAX_DATA`] or the link is closed; a full queue closes the link. Empty
    /// payloads are never delivered.
    pub fn send_to(&self, target: u8, bytes: &[u8]) -> io::Result<()> {
        self.0.send(&[target], bytes)
    }
}

pub struct ClientLink(Link<ClientIncoming>);

impl ClientLink {
    pub fn try_recv(&self) -> Option<ClientIncoming> {
        self.0.incoming.try_recv().ok()
    }

    /// Queues `bytes` for the host. Fails if `bytes` is over [`MAX_DATA`] or the link is
    /// closed; a full queue closes the link. Empty payloads are keepalives and never
    /// delivered.
    pub fn send(&self, bytes: &[u8]) -> io::Result<()> {
        self.0.send(&[], bytes)
    }
}

struct Link<T> {
    incoming: Receiver<T>,
    outgoing: SyncSender<Vec<u8>>,
    conn: Arc<Conn>,
}

/// The socket, shared by a link and its two threads.
struct Conn {
    stream: TcpStream,
    closed: AtomicBool,
}

impl Conn {
    /// Fails later writes and ends the reader: at once on Linux, where shutdown wakes a
    /// blocked read; on Windows, which keeps delivering data after a shutdown, at the next
    /// frame (the relay sends one at least every 5 s) or the idle timeout.
    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl<T: Send + 'static> Link<T> {
    fn spawn(
        stream: TcpStream,
        decode: fn(Vec<u8>) -> Option<T>,
        closed: fn(String) -> T,
    ) -> io::Result<Self> {
        stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let conn = Arc::new(Conn { stream, closed: AtomicBool::new(false) });
        let (in_tx, incoming) = mpsc::sync_channel(LINK_QUEUE);
        let (outgoing, out_rx) = mpsc::sync_channel::<Vec<u8>>(LINK_QUEUE);

        let reader = Arc::clone(&conn);
        let read_loop = move || {
            let error = loop {
                let frame = read_frame(&mut &reader.stream);
                if reader.closed.load(Ordering::Relaxed) {
                    break io::Error::new(ErrorKind::ConnectionAborted, "relay link closed");
                }
                match frame {
                    Ok(frame) if frame.is_empty() => {} // keepalive
                    Ok(frame) => {
                        if let Some(msg) = decode(frame) {
                            if in_tx.send(msg).is_err() {
                                return; // link dropped
                            }
                        }
                    }
                    Err(e) => break e,
                }
            };
            let _ = in_tx.send(closed(close_reason(&error)));
        };
        let writer = Arc::clone(&conn);
        let write_loop = move || {
            loop {
                let frame = match out_rx.recv_timeout(KEEPALIVE_INTERVAL) {
                    Ok(frame) => frame,
                    Err(RecvTimeoutError::Timeout) => vec![0; 4],
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                if (&writer.stream).write_all(&frame).is_err() {
                    break;
                }
            }
            // The reader then reports `Closed`.
            writer.close();
        };

        let spawned = thread::Builder::new()
            .name("relay-read".into())
            .spawn(read_loop)
            .and_then(|_| thread::Builder::new().name("relay-write".into()).spawn(write_loop));
        if let Err(e) = spawned {
            conn.close();
            return Err(e);
        }
        Ok(Self { incoming, outgoing, conn })
    }

    fn send(&self, prefix: &[u8], bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_DATA {
            return Err(io::Error::new(ErrorKind::InvalidInput, "payload over MAX_DATA"));
        }
        let frame = encode_frame(prefix, bytes)?;
        match self.outgoing.try_send(frame) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                // The reader then reports `Closed`.
                self.conn.close();
                Err(io::Error::new(ErrorKind::WouldBlock, "relay link backed up, closed"))
            }
            Err(TrySendError::Disconnected(_)) => {
                Err(io::Error::new(ErrorKind::NotConnected, "relay link closed"))
            }
        }
    }
}

fn decode_host(mut frame: Vec<u8>) -> Option<HostIncoming> {
    let from = *frame.first()?;
    if from == CONTROL {
        return match Control::parse(&frame[1..])? {
            Control::Joined(id) => Some(HostIncoming::Joined(id)),
            Control::Left(id) => Some(HostIncoming::Left(id)),
        };
    }
    frame.remove(0);
    Some(HostIncoming::Data { from, bytes: frame })
}

fn handshake<A: ToSocketAddrs>(
    addr: A,
    hello: &Handshake,
) -> Result<(TcpStream, String), RelayError> {
    let stream = connect(addr)?;
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    write_frame(&mut &stream, &hello.encode())?;
    let mut reader = Deadline::new(&stream, HANDSHAKE_TIMEOUT);
    let reply = loop {
        let frame = read_frame(&mut reader)?;
        if !frame.is_empty() {
            break frame;
        }
    };
    match parse_reply(&reply) {
        Some(Ok(value)) => Ok((stream, value.to_owned())),
        Some(Err(refusal)) => Err(RelayError::Refused(refusal)),
        None => Err(bad_reply().into()),
    }
}

fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<TcpStream> {
    let mut last = io::Error::new(ErrorKind::InvalidInput, "address resolved to nothing");
    for addr in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn bad_reply() -> io::Error {
    io::Error::new(ErrorKind::InvalidData, "unexpected reply from relay")
}

fn close_reason(e: &io::Error) -> String {
    match e.kind() {
        ErrorKind::UnexpectedEof => "connection closed".into(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut => "connection timed out".into(),
        _ => e.to_string(),
    }
}
