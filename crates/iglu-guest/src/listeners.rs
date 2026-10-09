//! What the workspace user has listening: from `/proc` on Linux, which also
//! tells which terminal session started it, and from `lsof` elsewhere, which
//! doesn't.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use iglu_domain::guest::ListenerReport;

/// A listening socket as `/proc/net/tcp` describes it.
#[derive(Debug, PartialEq, Eq)]
pub struct Socket {
    pub address: IpAddr,
    pub port: u16,
    pub uid: u32,
    pub inode: u64,
}

const LISTEN: &str = "0A";

/// The listening sockets in one of `/proc/net/tcp` or `/proc/net/tcp6`.
/// Lines that don't parse are skipped.
#[must_use]
pub fn parse_net_tcp(text: &str) -> Vec<Socket> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (local, state, uid, inode) = (
                fields.get(1)?,
                fields.get(3)?,
                fields.get(7)?,
                fields.get(9)?,
            );
            if *state != LISTEN {
                return None;
            }
            let (address, port) = local.split_once(':')?;
            Some(Socket {
                address: parse_address(address)?,
                port: u16::from_str_radix(port, 16).ok()?,
                uid: uid.parse().ok()?,
                inode: inode.parse().ok()?,
            })
        })
        .collect()
}

/// Addresses are written as 32-bit words, each in host (little-endian) order.
fn parse_address(hex: &str) -> Option<IpAddr> {
    let words: Vec<u32> = (0..hex.len() / 8)
        .map(|i| u32::from_str_radix(hex.get(i * 8..i * 8 + 8)?, 16).ok())
        .collect::<Option<_>>()?;
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    match bytes.len() {
        4 => Some(IpAddr::V4(Ipv4Addr::new(
            bytes[0], bytes[1], bytes[2], bytes[3],
        ))),
        16 => <[u8; 16]>::try_from(bytes)
            .ok()
            .map(|b| IpAddr::V6(Ipv6Addr::from(b))),
        _ => None,
    }
}

/// The `ZMX_SESSION` in a process's environment, if any.
#[must_use]
pub fn session_of(environ: &[u8]) -> Option<String> {
    environ
        .split(|b| *b == 0)
        .find_map(|entry| entry.strip_prefix(b"ZMX_SESSION="))
        .and_then(|value| std::str::from_utf8(value).ok())
        .map(str::to_owned)
}

/// Everything the current user is listening on, with the owning process.
#[must_use]
pub fn scan(proc: &Path) -> Vec<ListenerReport> {
    if !proc.join("net/tcp").exists() {
        return lsof();
    }
    let uid = rustix::process::getuid().as_raw();
    let sockets: Vec<Socket> = ["net/tcp", "net/tcp6"]
        .iter()
        .filter_map(|file| std::fs::read_to_string(proc.join(file)).ok())
        .flat_map(|text| parse_net_tcp(&text))
        .filter(|socket| socket.uid == uid)
        .collect();
    if sockets.is_empty() {
        return Vec::new();
    }
    let owners = owners(proc);
    sockets
        .into_iter()
        .map(|socket| {
            let pid = owners
                .iter()
                .find(|(inode, _)| *inode == socket.inode)
                .map(|(_, pid)| pid);
            let read =
                |name: &str| pid.and_then(|pid| std::fs::read(proc.join(pid).join(name)).ok());
            ListenerReport {
                port: socket.port,
                address: socket.address,
                process: read("comm")
                    .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
                    .unwrap_or_default(),
                session: read("environ").and_then(|environ| session_of(&environ)),
            }
        })
        .collect()
}

/// The same from `lsof`, for systems without `/proc`.
fn lsof() -> Vec<ListenerReport> {
    let uid = rustix::process::getuid().as_raw().to_string();
    std::process::Command::new("lsof")
        .args([
            "-a",
            "-nP",
            "-iTCP",
            "-sTCP:LISTEN",
            "-u",
            &uid,
            "-F",
            "pctn",
        ])
        .output()
        .map(|output| parse_lsof(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

/// Reads `lsof -F pctn`: a `p` line starts each process, `c` names it,
/// and each socket has a `t` (IPv4 or IPv6) and an `n` (address:port).
#[must_use]
pub fn parse_lsof(output: &str) -> Vec<ListenerReport> {
    let mut reports = Vec::new();
    let mut process = String::new();
    let mut six = false;
    for line in output.lines() {
        let (field, value) = line.split_at(line.len().min(1));
        match field {
            "p" => process.clear(),
            "c" => value.clone_into(&mut process),
            "t" => six = value == "IPv6",
            "n" => {
                let Some((host, port)) = value.rsplit_once(':') else {
                    continue;
                };
                let host = host.trim_start_matches('[').trim_end_matches(']');
                let address = match (host, six) {
                    ("*", false) => Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
                    ("*", true) => Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
                    (host, _) => host.parse().ok(),
                };
                if let (Some(address), Ok(port)) = (address, port.parse()) {
                    reports.push(ListenerReport {
                        port,
                        address,
                        process: process.clone(),
                        session: None,
                    });
                }
            }
            _ => {}
        }
    }
    reports
}

/// Socket inodes and the processes holding them, for this user's processes.
fn owners(proc: &Path) -> Vec<(u64, String)> {
    let Ok(entries) = std::fs::read_dir(proc) else {
        return Vec::new();
    };
    let mut owners = Vec::new();
    for entry in entries.flatten() {
        let pid = entry.file_name().to_string_lossy().into_owned();
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            if let Some(inode) = target
                .to_str()
                .and_then(|t| t.strip_prefix("socket:["))
                .and_then(|t| t.strip_suffix(']'))
                .and_then(|t| t.parse().ok())
            {
                owners.push((inode, pid.clone()));
            }
        }
    }
    owners
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines from a real /proc/net/tcp and tcp6, trimmed of trailing fields.
    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41234 1 0
   1: 00000000:1435 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41250 1 0
   2: 0100007F:0BB8 0100007F:D2C4 01 00000000:00000000 00:00000000 00000000  1000        0 41300 1 0
   3: garbage
";
    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 9001 1 0
";

    #[test]
    fn listening_sockets_parse_and_others_are_skipped() {
        let sockets = parse_net_tcp(TCP);
        assert_eq!(
            sockets
                .iter()
                .map(|s| (s.address.to_string(), s.port, s.uid, s.inode))
                .collect::<Vec<_>>(),
            [
                ("127.0.0.1".into(), 3000, 1000, 41234),
                ("0.0.0.0".into(), 5173, 1000, 41250)
            ]
        );
        let six = parse_net_tcp(TCP6);
        assert_eq!(
            six.iter()
                .map(|s| (s.address.to_string(), s.port))
                .collect::<Vec<_>>(),
            [("::1".into(), 8080)]
        );
    }

    #[test]
    fn lsof_listing_parses() {
        let output = "p501\ncnode\nf23\ntIPv4\nn127.0.0.1:3000\nf24\ntIPv6\nn*:5173\np777\ncpostgres\nf5\ntIPv6\nn[::1]:5432\n";
        let seen: Vec<(String, u16, String)> = parse_lsof(output)
            .into_iter()
            .map(|r| (r.address.to_string(), r.port, r.process))
            .collect();
        assert_eq!(
            seen,
            [
                ("127.0.0.1".into(), 3000, "node".into()),
                ("::".into(), 5173, "node".into()),
                ("::1".into(), 5432, "postgres".into()),
            ]
        );
    }

    #[test]
    fn the_session_comes_from_the_environment() {
        assert_eq!(
            session_of(b"HOME=/home/dev\0ZMX_SESSION=server\0TERM=xterm\0").as_deref(),
            Some("server")
        );
        assert_eq!(session_of(b"HOME=/home/dev\0"), None);
    }
}
