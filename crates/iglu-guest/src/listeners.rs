//! What the workspace user has listening: from `/proc` on Linux, and from
//! `lsof` and `ps` on a Mac, for the dev stack's local runtime. Both tell
//! which terminal session started each listener.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use iglu_domain::guest::{LOCAL_WORKSPACE, ListenerReport};

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

/// The same from `lsof`, for systems without `/proc`: the local runtime on
/// a Mac. Every process there is this user's, so a listener counts only when
/// it, or a process it descends from, carries this workspace's marker.
/// Ancestry matters because macOS hides system programs' environments, such
/// as `/usr/bin/nc`'s, while the terminal session that started one shows.
fn lsof() -> Vec<ListenerReport> {
    let sockets = lsof_sockets();
    let Ok(workspace) = std::env::var(LOCAL_WORKSPACE) else {
        return sockets.into_iter().map(|(_, report)| report).collect();
    };
    if sockets.is_empty() {
        return Vec::new();
    }
    let table = std::process::Command::new("ps")
        .args(["-wwEA", "-o", "pid=,ppid=,command="])
        .output()
        .map(|output| parse_ps(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();
    sockets
        .into_iter()
        .filter_map(|(pid, report)| {
            let found = placement(&table, pid);
            (found.workspace.as_deref() == Some(workspace.as_str())).then_some(ListenerReport {
                session: found.session,
                ..report
            })
        })
        .collect()
}

/// A process as `ps` shows it: its parent, and what its environment says
/// about where it runs, when the environment shows.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Process {
    pub parent: u32,
    pub placement: Placement,
}

/// Where a process runs: its workspace and terminal session.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Placement {
    pub workspace: Option<String>,
    pub session: Option<String>,
}

/// Reads `ps -wwEA -o pid=,ppid=,command=`: each line a process ID, its
/// parent's, then its command line and environment, space-separated. The
/// values of the variables read here never contain spaces. A session's own
/// `zmx attach <name>` process names it in its arguments, since `ZMX_SESSION`
/// is only in its children's environments.
#[must_use]
pub fn parse_ps(output: &str) -> ProcessTable {
    ProcessTable(
        output
            .lines()
            .filter_map(|line| {
                let mut words = line.split_whitespace().peekable();
                let pid = words.next()?.parse().ok()?;
                let parent = words.next()?.parse().ok()?;
                let mut placement = Placement::default();
                let program = words.next().unwrap_or_default();
                if Path::new(program)
                    .file_name()
                    .is_some_and(|name| name == "zmx")
                    && words.next_if(|w| *w == "attach" || *w == "a").is_some()
                {
                    if words.next_if_eq(&"--labels").is_some() {
                        words.next();
                    }
                    placement.session = words.next().map(str::to_owned);
                }
                for word in words {
                    if let Some(value) = word
                        .strip_prefix(LOCAL_WORKSPACE)
                        .and_then(|rest| rest.strip_prefix('='))
                    {
                        placement.workspace = Some(value.to_owned());
                    } else if let Some(value) = word.strip_prefix("ZMX_SESSION=") {
                        placement.session = Some(value.to_owned());
                    }
                }
                Some((pid, Process { parent, placement }))
            })
            .collect(),
    )
}

/// Every process `ps` showed, by ID.
#[derive(Debug, Default)]
pub struct ProcessTable(HashMap<u32, Process>);

/// Where `pid` runs: each of its workspace and session from the nearest of
/// it and its ancestors that shows one.
#[must_use]
pub fn placement(ProcessTable(table): &ProcessTable, pid: u32) -> Placement {
    let mut found = Placement::default();
    let mut current = pid;
    // A process table can't be deeper than it is long; this also ends a loop.
    for _ in 0..=table.len() {
        let Some(process) = table.get(&current) else {
            break;
        };
        if found.workspace.is_none() {
            found.workspace.clone_from(&process.placement.workspace);
        }
        if found.session.is_none() {
            found.session.clone_from(&process.placement.session);
        }
        if (found.workspace.is_some() && found.session.is_some()) || process.parent == current {
            break;
        }
        current = process.parent;
    }
    found
}

fn lsof_sockets() -> Vec<(u32, ListenerReport)> {
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
/// Each listener comes with its process's ID.
#[must_use]
pub fn parse_lsof(output: &str) -> Vec<(u32, ListenerReport)> {
    let mut reports = Vec::new();
    let mut pid = 0;
    let mut process = String::new();
    let mut six = false;
    for line in output.lines() {
        let (field, value) = line.split_at(line.len().min(1));
        match field {
            "p" => {
                process.clear();
                pid = value.parse().unwrap_or(0);
            }
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
                    reports.push((
                        pid,
                        ListenerReport {
                            port,
                            address,
                            process: process.clone(),
                            session: None,
                        },
                    ));
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
        let seen: Vec<(u32, String, u16, String)> = parse_lsof(output)
            .into_iter()
            .map(|(pid, r)| (pid, r.address.to_string(), r.port, r.process))
            .collect();
        assert_eq!(
            seen,
            [
                (501, "127.0.0.1".into(), 3000, "node".into()),
                (501, "::".into(), 5173, "node".into()),
                (777, "::1".into(), 5432, "postgres".into()),
            ]
        );
    }

    #[test]
    fn a_listener_is_placed_by_the_nearest_ancestor_that_shows_where_it_runs() {
        // zmx's session (10) shows its workspace in its environment and its
        // session in its arguments; the shell (11) and nc (12) under it don't
        // show their environments.
        let output = "    1     0 launchd\n   10     1 /nix/store/x-zmx/bin/zmx attach --labels k=v server sh -c nc IGLU_DEVHOST_WORKSPACE=iglu-a\n   11    10 /bin/sh -c nc\n   12    11 /usr/bin/nc -l 3000\n   20     1 postgres HOME=/u\n";
        let table = parse_ps(output);
        assert_eq!(
            placement(&table, 12),
            Placement {
                workspace: Some("iglu-a".into()),
                session: Some("server".into())
            }
        );
        assert_eq!(placement(&table, 20), Placement::default());
        assert_eq!(placement(&table, 99), Placement::default());
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
