//! The TCP ports the programs of a tab listen on (a dev server), from the tables of the kernel (`/proc/net/tcp`, `tcp6`):
//! one look at the listeners of the machine, IPv4 and IPv6, per tick for every pane, filtered by the processes of each
//! pane's job.

use std::{collections::HashMap, fs};

/// A listening socket: the process that owns it and its port.
pub type Listener = (u32, u16);

/// The state `TCP_LISTEN` in the tables of the kernel.
const LISTEN: &str = "0A";

/// The listening sockets of a table, as (inode, port). A row is: slot, local address:port (hex), remote address:port, state,
/// queues, timer, retransmits, user, timeout, inode.
fn parse(table: &str) -> Vec<(u64, u16)> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.get(3) != Some(&LISTEN) {
                return None;
            }
            let port = u16::from_str_radix(fields.get(1)?.rsplit(':').next()?, 16).ok()?;
            Some((fields.get(9)?.parse().ok()?, port))
        })
        .collect()
}

/// Every listening TCP socket of the machine, IPv4 and IPv6, that a process of the user holds.
pub fn listeners() -> Vec<Listener> {
    let mut sockets: HashMap<u64, u16> = HashMap::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = fs::read_to_string(table) {
            sockets.extend(parse(&text));
        }
    }
    if sockets.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let Ok(processes) = fs::read_dir("/proc") else { return found };
    for process in processes.flatten() {
        let Some(pid) = process.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else { continue };
        let Ok(descriptors) = fs::read_dir(process.path().join("fd")) else { continue };
        for descriptor in descriptors.flatten() {
            let Ok(target) = fs::read_link(descriptor.path()) else { continue };
            let inode = target.to_str().and_then(|text| text.strip_prefix("socket:[")).and_then(|text| text.strip_suffix(']')).and_then(|text| text.parse::<u64>().ok());
            if let Some(port) = inode.and_then(|inode| sockets.get(&inode)) {
                found.push((pid, *port));
            }
        }
    }
    found
}

/// The ports `pids` listen on, ascending, each once (a server on both families counts once).
pub fn ports_of(listeners: &[Listener], pids: &[u32]) -> Vec<u16> {
    let mut ports: Vec<u16> = listeners.iter().filter(|(pid, _)| pids.contains(pid)).map(|(_, port)| *port).collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// What opens a port in the browser.
pub fn url(port: u16) -> String {
    format!("http://localhost:{port}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_listening_rows_of_a_table_give_a_port() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000000000000000 100 0 0 10 0\n   1: 0100007F:A5C2 0100007F:1F90 01 00000000:00000000 00:00000000 00000000  1000        0 999 1 0000000000000000 20 4 30 10 -1\n";
        assert_eq!(parse(table), [(12345, 8080)]);
    }

    #[test]
    fn a_server_on_both_families_counts_once() {
        assert_eq!(ports_of(&[(7, 3000), (7, 3000), (8, 22)], &[7]), [3000]);
    }

    #[test]
    fn this_process_listening_is_found() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        assert!(ports_of(&listeners(), &[std::process::id()]).contains(&port));
    }
}
