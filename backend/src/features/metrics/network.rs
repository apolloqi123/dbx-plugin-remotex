//! Metrics support.

use super::*;

/// Per-interface traffic counters and B/s rates between the two snapshots.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfaceMetrics {
    pub name: String,
    /// Receive rate in bytes per second between the two samples.
    pub rx_rate: f64,
    /// Transmit rate in bytes per second between the two samples.
    pub tx_rate: f64,
    /// Cumulative received bytes at the second sample.
    pub rx_total: u64,
    /// Cumulative transmitted bytes at the second sample.
    pub tx_total: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NetCounters {
    pub(crate) rx: u64,
    pub(crate) tx: u64,
}

/// Parses one counter snapshot (either `/proc/net/dev` or `netstat -ibn`
/// text) into per-interface cumulative byte counters. The two formats are
/// distinguished per line, and `netstat` rows repeat per address so the
/// maximum per interface name wins (mirrors tiny-rdm's awk dedup).
pub(crate) fn parse_net_counters(text: &str) -> BTreeMap<String, NetCounters> {
    let mut counters: BTreeMap<String, NetCounters> = BTreeMap::new();
    for line in text.lines() {
        if let Some((name, counter)) = parse_net_line(line) {
            let entry = counters.entry(name).or_default();
            if counter.rx > entry.rx {
                entry.rx = counter.rx;
            }
            if counter.tx > entry.tx {
                entry.tx = counter.tx;
            }
        }
    }
    counters
}

/// Parses one line of either network-counter format into `(name, counters)`.
pub(crate) fn parse_net_line(line: &str) -> Option<(String, NetCounters)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Linux /proc/net/dev: `  eth0: rx_bytes packets … tx_bytes packets …`
    // (16 numeric columns, rx bytes at 0 and tx bytes at 8).
    if let Some((name, rest)) = trimmed.split_once(':') {
        let name = name.trim();
        if !name.is_empty() && !name.chars().any(char::is_whitespace) {
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if fields.len() >= 16 {
                if let (Ok(rx), Ok(tx)) = (fields[0].parse::<u64>(), fields[8].parse::<u64>()) {
                    return Some((name.to_string(), NetCounters { rx, tx }));
                }
            }
        }
    }
    // macOS/BSD netstat -ibn:
    // `Name Mtu Network Address Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll`.
    // Rows without an Address collapse to 10 fields, but `Coll` is always
    // last, `Obytes` second to last, and `Ibytes` three columns before it.
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    if fields.len() < 10 || fields[0] == "Name" {
        return None;
    }
    let tx = fields[fields.len() - 2].parse::<u64>().ok()?;
    let rx = fields[fields.len() - 5].parse::<u64>().ok()?;
    Some((fields[0].to_string(), NetCounters { rx, tx }))
}

/// Builds the per-interface rate view from the two snapshots: rates in B/s
/// over [`NET_SAMPLE_INTERVAL_SECS`], cumulative totals from the second
/// sample. Interfaces without any traffic stay hidden; results are sorted by
/// total rate (busiest first, name as tiebreaker).
pub fn parse_network_samples(output: &str) -> Vec<NetworkInterfaceMetrics> {
    let first = parse_net_counters(section_of(output, "--net--"));
    let second = parse_net_counters(section_of(output, "--net2--"));
    let mut interfaces: Vec<NetworkInterfaceMetrics> = second
        .into_iter()
        .map(|(name, current)| {
            let previous = first.get(&name).copied().unwrap_or(current);
            let rx_rate = current.rx.saturating_sub(previous.rx) as f64 / NET_SAMPLE_INTERVAL_SECS;
            let tx_rate = current.tx.saturating_sub(previous.tx) as f64 / NET_SAMPLE_INTERVAL_SECS;
            NetworkInterfaceMetrics {
                name,
                rx_rate,
                tx_rate,
                rx_total: current.rx,
                tx_total: current.tx,
            }
        })
        .filter(|net| {
            net.rx_total > 0 || net.tx_total > 0 || net.rx_rate > 0.0 || net.tx_rate > 0.0
        })
        .collect();
    interfaces.sort_by(|a, b| {
        let a_rate = a.rx_rate + a.tx_rate;
        let b_rate = b.rx_rate + b.tx_rate;
        b_rate
            .partial_cmp(&a_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
    interfaces
}
