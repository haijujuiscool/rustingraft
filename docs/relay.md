# Rusting Raft relay server

`raft-relay` lets up to 8 players play together without anyone port-forwarding their own
PC. One machine (a Linux home server) runs the relay on one TCP port. A host opens a
**room** and gets a 6-character room code; friends join with that code. The relay forwards
bytes between the host and its clients and never looks inside them.

It is a single std-only Rust binary: no dependencies, no `unsafe`, no files written, no
shell, no config file.

## Build (Linux)

Install Rust with [rustup](https://rustup.rs) (stable is fine), then from the repository:

```sh
cargo build --release -p raft_relay      # -> target/release/raft-relay
```

If the workspace does not load on the server (the game's root `Cargo.toml` needs the engine
checked out next to it at `../RustingEngine`), build the relay crate on its own. It has no dependencies, so
copy it out of the workspace and build there:

```sh
cp -r RustingRaft/relay ~/raft-relay-src
cd ~/raft-relay-src
cargo build --release                     # -> ~/raft-relay-src/target/release/raft-relay
cargo test                                # optional: codec, limits, loopback tests
```

## Install as a systemd service

```sh
sudo install -m 0755 target/release/raft-relay /usr/local/bin/raft-relay
sudo install -m 0644 deploy/rusting-relay.service /etc/systemd/system/rusting-relay.service
sudo systemctl daemon-reload
sudo systemctl enable --now rusting-relay
systemctl status rusting-relay
journalctl -u rusting-relay -f            # live log
systemd-analyze security rusting-relay    # sandbox score
```

The unit runs the relay as a throwaway user (`DynamicUser=yes`) with no capabilities, a
read-only filesystem, no home directories, private `/tmp` and `/dev`, only IPv4/IPv6
sockets, the `@system-service` syscall filter, 256 MB of memory and 1100 tasks at most. It
restarts on failure.

### Flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `--bind` | `0.0.0.0:27777` | Listen address. `[::]:27777` listens on IPv6 and IPv4. |
| `--max-rooms` | `64` | Open rooms at once. Extra hosts get `ERR full`. |
| `--max-clients-per-room` | `7` | Clients per room besides the host (1..7). |
| `--max-conns` | `512` | Open connections in total. |
| `--max-conns-per-ip` | `8` | Open connections per IP (IPv6: per /48). |
| `--handshake-timeout` | `5` | Seconds a new connection has to send its handshake. |
| `--idle-timeout` | `30` | Seconds without any frame before a connection is dropped (min 10). |

To change flags, override `ExecStart`:

```sh
sudo systemctl edit rusting-relay
```

```ini
[Service]
ExecStart=
ExecStart=/usr/local/bin/raft-relay --bind 0.0.0.0:27777 --max-rooms 8
```

Keep `--max-conns` under half the unit's `TasksMax=1100`: each connection has a reader
thread and, once in a room, a writer thread.

The default `--bind 0.0.0.0:27777` is IPv4 only. Only listen on IPv6 if your players need
it: one person with a free tunnel-broker /48 counts as one address for every limit, but
anyone holding several /48s counts as several.

## Network setup

1. Give the server a fixed LAN address (a DHCP reservation in the router).
2. In the router, forward **TCP port 27777** (external) to the server's LAN address, port
   27777. TCP only; no UDP is needed.
3. Open the port in the server's firewall. With ufw:

   ```sh
   sudo ufw allow 27777/tcp comment 'Rusting Raft relay'
   # or, to also have the kernel throttle connection floods per IP
   # (6 new connections per 30 s per IP; fine for players, each holds one connection):
   sudo ufw limit 27777/tcp comment 'Rusting Raft relay'
   ```

4. Players need your public address. If your ISP changes it, use a dynamic DNS name.

Test from outside your network (for example a phone hotspot): `nc -vz your.address 27777`.

## How players use it

- **Host:** main menu -> *Host online* -> enter the relay address (for example
  `relay.example.net:27777`). The game shows a room code like `K7M2QX`. Send it to your
  friends.
- **Friends:** main menu -> *Join online* -> the same relay address, the room code and a
  player name. Up to 7 friends per room.
- Codes are not case-sensitive and never contain `0`, `O`, `1`, `I` or `L`.
- The room lives as long as the host stays connected. When the host quits, everyone in the
  room is disconnected.
- Join errors: `no_room` (typo, or the host left), `full` (7 clients already),
  `rate_limited` (more than 10 join attempts in a minute from your address; wait 60 s),
  `bad_request` (malformed code or an incompatible game version).
- Host errors: `full` (all rooms in use), `rate_limited` (more than 10 rooms opened in a
  minute from your address; wait 60 s).

## What it logs

To stderr (journald): rooms opened/closed (with the room code, the host's IP and the room
count), clients joining/leaving with the client count, refused joins and hosts, connections
dropped for bandwidth, and every 10 s a count of connections refused over the limits.
Single connections and rate-limited attempts are not logged, so a flood cannot flood the
journal. It never logs or parses game data.

## What it defends against

- **Room code guessing.** 31^6 (about 887 million) codes, and each IP (IPv6: each /48) gets
  10 join attempts per minute, then 60 s of `ERR rate_limited`. Finding one of 64 open rooms
  takes about 14 million guesses on average, roughly 1.4 million IP-minutes. The relay
  tracks at most 16,384 addresses at once; while a flood from more addresses than that is
  under way, joins and room opens from addresses it is not tracking yet are refused with
  `ERR rate_limited`.
- **Room churn.** Each IP can open 10 rooms per minute, then gets 60 s of
  `ERR rate_limited`.
- **Connection exhaustion.** At most 512 connections, 8 per IP. A connection that does not
  finish its handshake within 5 s in total is dropped, even if it trickles bytes
  (slowloris), as is one silent for 30 s.
- **Bandwidth abuse by a connection.** Each connection is limited to 256 KiB/s sustained
  with a 512 KiB burst; going over drops the connection.
- **Malformed input.** Frames over 64 KiB, truncated frames, garbage handshakes and unknown
  targets are rejected or dropped. All reads are checked; bad input cannot panic it.
- **Stalled receivers.** Every connection has its own send queue and writer thread, so no
  peer ever waits on another's socket. A peer that falls 256 KiB behind (or whose socket
  takes no data for 5 s) is dropped; the rest of its room carries on. A host that falls
  behind closes its room.
- **Exposed IPs between players.** Players only ever talk to the relay, so they never see
  each other's IP addresses.
- **Damage if the process were compromised.** The systemd sandbox above: no privileges, no
  writable filesystem, no other socket families, capped memory and tasks.

## What it cannot defend against

- **Volumetric DDoS.** Anyone with a botnet or a booter service can fill a home internet
  line with junk traffic before it ever reaches the server. No software on that machine can
  stop this; only your ISP or an upstream filter can. If it happens: stop the port forward,
  get a new public IP (often a router reconnect), and keep the address among friends. For
  more exposure, run the relay on a VPS with DDoS filtering, or skip port forwarding and
  play over a private VPN (WireGuard, Tailscale).
- **SYN floods** are the kernel's job (SYN cookies, on by default), not the relay's.
- **Many attacking IPs.** The limits bound memory and CPU, not availability: enough
  distinct addresses can still fill the 512-connection pool, and real players are then
  refused until the attack stops. Opening a room needs no secret, so 8 addresses (8 rooms
  each) can hold all 64 rooms; lower `--max-conns-per-ip` or raise `--max-rooms` if that
  happens.
- **Using the relay to fill your upload.** A host may send 256 KiB/s and every byte it
  broadcasts goes to up to 7 clients: about 1.75 MiB/s (15 Mbit/s) of upload per room. Two
  attacking addresses can open enough rooms to fill a typical home uplink. Size
  `--max-rooms` and `--max-conns-per-ip` to your upload speed; normal games use a small
  fraction of the cap.
- **Eavesdropping and tampering.** Traffic is plain TCP with no encryption or
  authentication. The room code is the only secret, and anyone who has it can join. Do not
  send anything private through it.
- **Misbehaving players.** Someone with the code can send anything to the host. The host
  game must treat every client message as untrusted input.

## Protocol reference

Frames are a `u32` big-endian length and that many bytes (at most 65,536). An empty frame
is a keepalive; both sides send one after 5 s of silence.

| Direction | Payload |
| --- | --- |
| first frame, host | `RRLY1 HOST` -> `OK <room code>` or `ERR <full / rate_limited>` |
| first frame, client | `RRLY1 JOIN <code>` -> `OK <id 1..7>` or `ERR <full / no_room / rate_limited / bad_request>` |
| client -> relay | data, forwarded to the host as `<id byte>` + data |
| host -> relay | `<target byte>` + data; target 0 = all clients, 1..7 = one client |
| relay -> host | `0xFF` + `JOINED <id>` or `LEFT <id>` |

## Upgrading and removing

```sh
sudo install -m 0755 target/release/raft-relay /usr/local/bin/raft-relay
sudo systemctl restart rusting-relay      # drops every open room

sudo systemctl disable --now rusting-relay
sudo rm /etc/systemd/system/rusting-relay.service /usr/local/bin/raft-relay
sudo systemctl daemon-reload
```
