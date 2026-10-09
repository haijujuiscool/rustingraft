//! `raft-relay`: the Rusting Raft relay server.
#![forbid(unsafe_code)]

use raft_relay::{Config, Server, MAX_CLIENT_ID};
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "usage: raft-relay [--bind 0.0.0.0:27777] [--max-rooms 64] \
[--max-clients-per-room 7] [--max-conns 512] [--max-conns-per-ip 8] \
[--handshake-timeout 5] [--idle-timeout 30]";

fn main() -> ExitCode {
    let config = match parse_args(std::env::args().skip(1)) {
        Ok(Some(config)) => config,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(msg) => {
            eprintln!("raft-relay: {msg}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let summary = format!(
        "rooms {}, clients/room {}, conns {}, conns/ip {}, handshake {}s, idle {}s",
        config.max_rooms,
        config.max_clients_per_room,
        config.max_conns,
        config.max_conns_per_ip,
        config.handshake_timeout.as_secs(),
        config.idle_timeout.as_secs(),
    );
    let bind = config.bind;
    let server = match Server::bind(config) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("raft-relay: cannot listen on {bind}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let addr = server.local_addr().map_or(bind.to_string(), |a| a.to_string());
    eprintln!("raft-relay listening on {addr} ({summary})");
    server.run()
}

/// `Ok(None)` means help was asked for.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Config>, String> {
    let mut config = Config::default();
    while let Some(flag) = args.next() {
        if flag == "-h" || flag == "--help" {
            return Ok(None);
        }
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let v = value.as_str();
        match flag.as_str() {
            "--bind" => {
                config.bind = v.parse().map_err(|_| format!("--bind wants ip:port, got {v}"))?
            }
            "--max-rooms" => config.max_rooms = number(&flag, v, 1, 100_000)?,
            "--max-clients-per-room" => {
                config.max_clients_per_room = number(&flag, v, 1, MAX_CLIENT_ID.into())? as u8
            }
            "--max-conns" => config.max_conns = number(&flag, v, 1, 100_000)?,
            "--max-conns-per-ip" => config.max_conns_per_ip = number(&flag, v, 1, 100_000)?,
            "--handshake-timeout" => config.handshake_timeout = secs(&flag, v, 1, 60)?,
            // Peers send keepalives every 5 s, so anything shorter would drop them.
            "--idle-timeout" => config.idle_timeout = secs(&flag, v, 10, 3600)?,
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    Ok(Some(config))
}

fn number(flag: &str, value: &str, min: usize, max: usize) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(n) if (min..=max).contains(&n) => Ok(n),
        _ => Err(format!("{flag} wants a whole number from {min} to {max}, got {value}")),
    }
}

fn secs(flag: &str, value: &str, min: usize, max: usize) -> Result<Duration, String> {
    number(flag, value, min, max).map(|n| Duration::from_secs(n as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Config>, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn flags() {
        let c = parse(&["--bind", "127.0.0.1:9000", "--max-rooms", "3", "--idle-timeout", "60"])
            .unwrap()
            .unwrap();
        assert_eq!(c.bind.port(), 9000);
        assert_eq!(c.max_rooms, 3);
        assert_eq!(c.idle_timeout, Duration::from_secs(60));
        assert_eq!(c.max_conns, 512);
        assert!(parse(&["--help"]).unwrap().is_none());
        for bad in [
            &["--max-clients-per-room", "8"][..],
            &["--max-rooms", "0"],
            &["--max-rooms", "-1"],
            &["--max-conns"],
            &["--bind", "nowhere"],
            &["--idle-timeout", "5"],
            &["--frobnicate", "1"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }
}
