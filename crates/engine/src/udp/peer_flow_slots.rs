use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use iroh::EndpointId;

/// Live flows each peer holds on one Serve service, across all of the peer's
/// connections (the per-peer cap is per peer, not per connection).
///
/// Quarantined sockets do not count here; they only hold a service slot.
#[derive(Debug, Default)]
pub struct PeerFlowSlots {
    // Never held across an await.
    held: Mutex<HashMap<EndpointId, usize>>,
}

impl PeerFlowSlots {
    /// Reserve one of `peer`'s `max` flow slots, or `None` when the peer
    /// already holds `max` live flows. The slot is released when the returned
    /// guard drops.
    pub fn try_reserve(self: &Arc<Self>, peer: EndpointId, max: usize) -> Option<PeerFlowSlot> {
        let mut held = self.lock();
        let count = held.entry(peer).or_default();
        if *count >= max {
            if *count == 0 {
                held.remove(&peer);
            }
            return None;
        }
        *count += 1;
        Some(PeerFlowSlot {
            slots: self.clone(),
            peer,
        })
    }

    fn release(&self, peer: EndpointId) {
        let mut held = self.lock();
        if let Some(count) = held.get_mut(&peer) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                held.remove(&peer);
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<EndpointId, usize>> {
        // The map stays consistent even if a holder panicked: every update
        // is a single step.
        self.held.lock().unwrap_or_else(|p| p.into_inner())
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

    #[test]
    fn a_peer_is_refused_past_its_cap_while_another_peer_is_not() {
        let slots = Arc::new(PeerFlowSlots::default());
        let (a, b) = (peer(), peer());
        let _a1 = slots.try_reserve(a, 2).expect("first slot");
        let _a2 = slots.try_reserve(a, 2).expect("second slot");
        assert!(slots.try_reserve(a, 2).is_none(), "third slot over the cap");
        assert!(slots.try_reserve(b, 2).is_some(), "other peer unaffected");
    }

    #[test]
    fn dropping_a_slot_frees_it_and_forgets_an_idle_peer() {
        let slots = Arc::new(PeerFlowSlots::default());
        let a = peer();
        let slot = slots.try_reserve(a, 1).expect("slot");
        assert!(slots.try_reserve(a, 1).is_none());
        drop(slot);
        assert!(slots.lock().is_empty(), "no entry left for an idle peer");
        assert!(slots.try_reserve(a, 1).is_some(), "slot reusable");
    }

    #[test]
    fn a_zero_cap_refuses_without_keeping_an_entry() {
        let slots = Arc::new(PeerFlowSlots::default());
        assert!(slots.try_reserve(peer(), 0).is_none());
        assert!(slots.lock().is_empty());
    }
}
