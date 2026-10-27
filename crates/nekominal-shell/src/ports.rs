//! The TCP ports the programs of a tab listen on (a dev server), from the tables of the IP helper: one look at the
//! listeners of the machine, IPv4 and IPv6, per tick for every pane, filtered by the processes of each pane's job.

use windows::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, TCP_TABLE_OWNER_PID_LISTENER};

/// `AF_INET` and `AF_INET6`, without the WinSock bindings for two numbers.
const AF_INET: u32 = 2;
const AF_INET6: u32 = 23;
/// `MIB_TCPROW_OWNER_PID`: six u32 (state, address, port, remote address, remote port, pid).
const ROW_V4: usize = 24;
/// `MIB_TCP6ROW_OWNER_PID`: address (16), scope, port, remote address (16), remote scope, remote port, state, pid.
const ROW_V6: usize = 56;

/// A listening socket: the process that owns it and its port.
pub type Listener = (u32, u16);

fn u32_at(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(at..at + 4)?.try_into().ok()?))
}

/// The port of a row: the low two bytes of its DWORD, in network order.
fn port(dw: u32) -> u16 {
    u16::from_be_bytes([dw as u8, (dw >> 8) as u8])
}

/// The rows of a `MIB_TCPTABLE_OWNER_PID` (`v6` false) or `MIB_TCP6TABLE_OWNER_PID` as the API writes it: a count,
/// then the rows. A count larger than the buffer gives the rows that fit.
fn parse(buf: &[u8], v6: bool) -> Vec<Listener> {
    let (size, port_at, pid_at) = if v6 { (ROW_V6, 20, 52) } else { (ROW_V4, 8, 20) };
    let count = u32_at(buf, 0).unwrap_or(0) as usize;
    (0..count)
        .map_while(|i| {
            let row = 4 + i * size;
            Some((u32_at(buf, row + pid_at)?, port(u32_at(buf, row + port_at)?)))
        })
        .collect()
}

/// The table of one family, asked for again with more room while the listeners grow in between.
fn table(family: u32) -> Vec<Listener> {
    let mut size = 0u32;
    for _ in 0..3 {
        // u32 words keep the buffer aligned for the table.
        let mut buf = vec![0u32; (size as usize).div_ceil(4)];
        let ptr = (!buf.is_empty()).then(|| buf.as_mut_ptr().cast());
        // SAFETY: `size` is the length of `buf` in bytes, and the API writes no more than that.
        let error = unsafe { GetExtendedTcpTable(ptr, &mut size, false, family, TCP_TABLE_OWNER_PID_LISTENER, 0) };
        if error == 0 && ptr.is_some() {
            let bytes: Vec<u8> = buf.iter().flat_map(|w| w.to_le_bytes()).collect();
            return parse(&bytes, family == AF_INET6);
        }
        // ERROR_INSUFFICIENT_BUFFER (122) set `size`; anything else is no table.
        if error != 0 && error != 122 {
            break;
        }
    }
    Vec::new()
}

/// Every listening TCP socket of the machine, IPv4 and IPv6.
pub fn listeners() -> Vec<Listener> {
    let mut all = table(AF_INET);
    all.extend(table(AF_INET6));
    all
}

/// The ports `pids` listen on, ascending, each once (a server on both families counts once).
pub fn ports_of(listeners: &[Listener], pids: &[u32]) -> Vec<u16> {
    let mut ports: Vec<u16> = listeners.iter().filter(|(pid, _)| pids.contains(pid)).map(|(_, port)| *port).collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// What opens a port in the browser.
#[allow(dead_code, reason = "for the badge, which is not drawn yet")]
pub fn url(port: u16) -> String {
    format!("http://localhost:{port}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(buf: &mut Vec<u8>, value: u32) {
        buf.extend(value.to_le_bytes());
    }

    /// A port as the API stores it: network order in the low two bytes.
    fn raw_port(port: u16) -> u32 {
        u16::from_ne_bytes(port.to_be_bytes()) as u32
    }

    #[test]
    fn the_rows_of_an_ipv4_table_give_their_pid_and_port() {
        let mut buf = Vec::new();
        word(&mut buf, 2);
        for (pid, p) in [(4242u32, 5173u16), (7, 80)] {
            for value in [2, 0x0100_007f, raw_port(p), 0, 0, pid] {
                word(&mut buf, value);
            }
        }
        assert_eq!(parse(&buf, false), [(4242, 5173), (7, 80)]);
        assert_eq!(parse(&buf[..buf.len() - 4], false), [(4242, 5173)], "a cut row is left out");
        assert!(parse(&[], false).is_empty());
    }

    #[test]
    fn the_rows_of_an_ipv6_table_give_their_pid_and_port() {
        let mut buf = Vec::new();
        word(&mut buf, 1);
        buf.extend([0u8; 16]);
        word(&mut buf, 0);
        word(&mut buf, raw_port(3000));
        buf.extend([0u8; 16]);
        for value in [0, 0, 2, 99] {
            word(&mut buf, value);
        }
        assert_eq!(buf.len(), 4 + ROW_V6);
        assert_eq!(parse(&buf, true), [(99, 3000)]);
    }

    #[test]
    fn only_the_ports_of_the_job_count_each_once() {
        let all = [(10, 8080), (11, 5173), (10, 8080), (12, 443), (10, 3000)];
        assert_eq!(ports_of(&all, &[10, 11]), [3000, 5173, 8080]);
        assert!(ports_of(&all, &[]).is_empty());
        assert_eq!(url(5173), "http://localhost:5173");
    }

    #[test]
    fn a_port_this_process_listens_on_is_found() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        assert!(ports_of(&listeners(), &[std::process::id()]).contains(&port));
    }
}
