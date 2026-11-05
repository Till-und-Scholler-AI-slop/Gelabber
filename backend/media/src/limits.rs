//! Caps on `/ws` sockets that have not joined a room (#286).
//!
//! A pending socket holds no ticket yet, so anyone can open one. Each client
//! address (`gelabber_shared::client_addr`) may hold `MEDIA_PENDING_PER_IP`
//! of them, and the whole SFU `MEDIA_PENDING_MAX` as a safety valve for file
//! descriptors. A per-address cap alone stops one host; the valve stops many
//! hosts, and is set high so one host cannot fill it and fail everyone's
//! join. The join deadline bounds how long a slot is held. A joined socket
//! gives its slot back: it holds a consumed ticket, which the API rations.
//! A socket that leaves takes a slot again for the time until it rejoins.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// This client address holds its share of pending sockets.
    PerIp,
    /// The SFU holds `MEDIA_PENDING_MAX` pending sockets.
    Total,
}

pub struct PendingSockets {
    per_ip: usize,
    max: usize,
    held: Mutex<Held>,
    refused_per_ip: AtomicU64,
    refused_total: AtomicU64,
}

#[derive(Default)]
struct Held {
    by_ip: HashMap<String, usize>,
    total: usize,
}

/// One pending socket. Dropping it gives the slot back.
pub struct PendingSlot {
    sockets: Arc<PendingSockets>,
    key: String,
}

impl PendingSockets {
    pub fn new(per_ip: usize, max: usize) -> Arc<Self> {
        Arc::new(Self {
            per_ip,
            max,
            held: Mutex::default(),
            refused_per_ip: AtomicU64::new(0),
            refused_total: AtomicU64::new(0),
        })
    }

    pub fn acquire(self: &Arc<Self>, key: &str) -> Result<PendingSlot, Refused> {
        let mut held = self.held.lock().unwrap_or_else(|err| err.into_inner());
        let mine = held.by_ip.get(key).copied().unwrap_or(0);
        let refused = if mine >= self.per_ip {
            Some((Refused::PerIp, &self.refused_per_ip))
        } else if held.total >= self.max {
            Some((Refused::Total, &self.refused_total))
        } else {
            None
        };
        if let Some((reason, counter)) = refused {
            counter.fetch_add(1, Ordering::Relaxed);
            return Err(reason);
        }
        held.by_ip.insert(key.to_owned(), mine + 1);
        held.total += 1;
        Ok(PendingSlot {
            sockets: Arc::clone(self),
            key: key.to_owned(),
        })
    }

    pub fn held(&self) -> usize {
        self.held
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .total
    }

    pub fn metrics_text(&self) -> String {
        format!(
            "# HELP gelabber_media_pending_sockets /ws sockets that have not joined a room.\n\
             # TYPE gelabber_media_pending_sockets gauge\n\
             gelabber_media_pending_sockets {}\n\
             # HELP gelabber_media_refused_sockets_total /ws upgrades refused by the pending-socket caps.\n\
             # TYPE gelabber_media_refused_sockets_total counter\n\
             gelabber_media_refused_sockets_total{{reason=\"per_ip\"}} {}\n\
             gelabber_media_refused_sockets_total{{reason=\"total\"}} {}\n",
            self.held(),
            self.refused_per_ip.load(Ordering::Relaxed),
            self.refused_total.load(Ordering::Relaxed),
        )
    }

    fn release(&self, key: &str) {
        let mut held = self.held.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(count) = held.by_ip.get_mut(key) {
            *count -= 1;
            if *count == 0 {
                held.by_ip.remove(key);
            }
            held.total -= 1;
        }
    }
}

impl Drop for PendingSlot {
    fn drop(&mut self) {
        self.sockets.release(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_address_gets_its_share() {
        let sockets = PendingSockets::new(2, 100);
        let _a1 = sockets.acquire("a").unwrap();
        let a2 = sockets.acquire("a").unwrap();
        assert_eq!(sockets.acquire("a").err(), Some(Refused::PerIp));
        let _b = sockets
            .acquire("b")
            .expect("another address is not affected");
        drop(a2);
        let _a3 = sockets.acquire("a").expect("a released slot is free again");
        assert_eq!(sockets.held(), 3);
    }

    #[test]
    fn the_valve_caps_all_addresses_together() {
        let sockets = PendingSockets::new(2, 3);
        let slots: Vec<_> = ["a", "b", "c"]
            .iter()
            .map(|key| sockets.acquire(key).unwrap())
            .collect();
        assert_eq!(sockets.acquire("d").err(), Some(Refused::Total));
        drop(slots);
        assert_eq!(sockets.held(), 0);
        assert!(sockets.acquire("d").is_ok());
    }

    #[test]
    fn released_addresses_leave_no_entry() {
        let sockets = PendingSockets::new(4, 4);
        drop(sockets.acquire("a").unwrap());
        assert!(sockets.held.lock().unwrap().by_ip.is_empty());
    }

    #[test]
    fn refusals_are_counted_by_reason() {
        let sockets = PendingSockets::new(1, 2);
        let _a = sockets.acquire("a").unwrap();
        let _ = sockets.acquire("a");
        let _b = sockets.acquire("b").unwrap();
        let _ = sockets.acquire("c");
        let text = sockets.metrics_text();
        assert!(
            text.contains("gelabber_media_pending_sockets 2\n"),
            "{text}"
        );
        assert!(
            text.contains("gelabber_media_refused_sockets_total{reason=\"per_ip\"} 1\n"),
            "{text}"
        );
        assert!(
            text.contains("gelabber_media_refused_sockets_total{reason=\"total\"} 1\n"),
            "{text}"
        );
    }
}
