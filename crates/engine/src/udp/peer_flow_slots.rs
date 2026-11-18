use std::collections::HashMap;
use std::sync::Arc;

use iroh::EndpointId;

use crate::fault::Fault;
use crate::locked::{Locked, Poisoned};

/// Live flows each peer holds on one Serve service, across all of the peer's
/// connections (the per-peer cap is per peer, not per connection).
///
/// Quarantined sockets do not count here; they only hold a service slot.
///
/// The counts are shared by every connection of the service, so a poisoned
/// lock raises a role fault.
#[derive(Debug)]
pub struct PeerFlowSlots {
    held: Locked<HashMap<EndpointId, usize>>,
    fault: Fault,
}

impl PeerFlowSlots {
    pub(crate) fn new(fault: Fault) -> Self {
        Self {
            held: Locked::default(),
            fault,
        }
    }

    /// Reserve one of `peer`'s `max` flow slots, or `None` when the peer
    /// already holds `max` live flows. The slot is released when the returned
    /// guard drops.
    ///
    /// # Errors
    ///
    /// [`Poisoned`] when the counts are poisoned; the role fault is raised.
    pub(crate) fn try_reserve(
        self: &Arc<Self>,
        peer: EndpointId,
        max: usize,
    ) -> Result<Option<PeerFlowSlot>, Poisoned> {
        let reserved = self.held.with(|held| {
            let count = held.entry(peer).or_default();
            if *count >= max {
                if *count == 0 {
                    held.remove(&peer);
                }
                return false;
            }
            *count += 1;
            true
        });
        let reserved = self.check(reserved)?;
        Ok(reserved.then(|| PeerFlowSlot {
            slots: self.clone(),
            peer,
        }))
    }

    fn release(&self, peer: EndpointId) {
        let released = self.held.with(|held| {
            if let Some(count) = held.get_mut(&peer) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    held.remove(&peer);
                }
            }
        });
        // Drop cannot propagate: the fault reports it.
        let _ = self.check(released);
    }

    fn check<T>(&self, result: Result<T, Poisoned>) -> Result<T, Poisoned> {
        result.inspect_err(|poisoned| {
            self.fault
                .raise(anyhow::Error::new(*poisoned).context("udp per-peer flow slots"));
        })
    }

    #[cfg(test)]
    fn held_peers(&self) -> usize {
        self.held.with(|held| held.len()).unwrap()
    }
}

/// One live flow's slot under its peer's cap; released on drop, so every way
/// a flow ends (close, idle expiry, connection drop, abort) gives it back.
#[derive(Debug)]
pub struct PeerFlowSlot {
    slots: Arc<PeerFlowSlots>,
    peer: EndpointId,
}

impl Drop for PeerFlowSlot {
    fn drop(&mut self) {
        self.slots.release(self.peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> EndpointId {
        iroh::SecretKey::generate().public()
    }

    fn slots() -> Arc<PeerFlowSlots> {
        Arc::new(PeerFlowSlots::new(Fault::detached()))
    }

    #[test]
    fn a_peer_is_refused_past_its_cap_while_another_peer_is_not() {
        let slots = slots();
        let (a, b) = (peer(), peer());
        let _a1 = slots.try_reserve(a, 2).unwrap().expect("first slot");
        let _a2 = slots.try_reserve(a, 2).unwrap().expect("second slot");
        assert!(
            slots.try_reserve(a, 2).unwrap().is_none(),
            "third slot over the cap"
        );
        assert!(
            slots.try_reserve(b, 2).unwrap().is_some(),
            "other peer unaffected"
        );
    }

    #[test]
    fn dropping_a_slot_frees_it_and_forgets_an_idle_peer() {
        let slots = slots();
        let a = peer();
        let slot = slots.try_reserve(a, 1).unwrap().expect("slot");
        assert!(slots.try_reserve(a, 1).unwrap().is_none());
        drop(slot);
        assert!(slots.held_peers() == 0, "no entry left for an idle peer");
        assert!(slots.try_reserve(a, 1).unwrap().is_some(), "slot reusable");
    }

    #[test]
    fn poisoned_counts_refuse_with_an_error_and_raise_a_role_fault() {
        let (fault, mut faults) = crate::fault::channel();
        let slots = Arc::new(PeerFlowSlots::new(fault));
        let poisoner = slots.clone();
        let _ = std::thread::spawn(move || {
            poisoner.held.with(|_| panic!("poison the counts")).ok();
        })
        .join();
        assert_eq!(slots.try_reserve(peer(), 1).err(), Some(Poisoned));
        assert!(faults.take().is_some(), "the role fault is raised");
    }

    #[test]
    fn a_zero_cap_refuses_without_keeping_an_entry() {
        let slots = slots();
        assert!(slots.try_reserve(peer(), 0).unwrap().is_none());
        assert!(slots.held_peers() == 0);
    }
}
