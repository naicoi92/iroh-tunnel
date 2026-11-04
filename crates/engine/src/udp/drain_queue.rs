use bytes::Bytes;

/// Run one flow's writer: hand each queued datagram to `sink` in order, until
/// every [`FlowQueue`] handle is dropped.
///
/// `sink` may wait (a slow target); only this flow's queue backs up behind it.
pub async fn drain_queue<F, Fut>(mut rx: tokio::sync::mpsc::Receiver<Bytes>, mut sink: F)
where
    F: FnMut(Bytes) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    while let Some(datagram) = rx.recv().await {
        sink(datagram).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::udp::{FlowQueue, Push};

    #[tokio::test]
    async fn the_flow_writer_drains_its_queue_in_order_through_the_sink() {
        let (queue, rx) = FlowQueue::new(4);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let writer = tokio::spawn(drain_queue(rx, move |datagram: Bytes| {
            let sink_seen = sink_seen.clone();
            async move { sink_seen.lock().unwrap().push(datagram) }
        }));
        for word in [&b"a"[..], b"b", b"c"] {
            assert_eq!(queue.push(Bytes::copy_from_slice(word)), Push::Queued);
        }
        drop(queue);
        writer.await.unwrap();
        let got: Vec<_> = seen.lock().unwrap().iter().map(|b| b.to_vec()).collect();
        assert_eq!(got, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[tokio::test]
    async fn a_stuck_sink_fills_only_its_own_queue_and_never_blocks_the_producer() {
        let gate = std::sync::Arc::new(tokio::sync::Notify::new());
        let (stuck, rx) = FlowQueue::new(2);
        let sink_gate = gate.clone();
        let writer = tokio::spawn(drain_queue(rx, move |_datagram: Bytes| {
            let sink_gate = sink_gate.clone();
            async move { sink_gate.notified().await }
        }));
        // The sink holds the first datagram, two more fill the queue, the rest
        // are refused at once: the producer never waits on the stuck flow.
        let mut full = 0;
        for _ in 0..10 {
            if stuck.push(Bytes::from_static(b"x")) == Push::Full {
                full += 1;
            }
            tokio::task::yield_now().await;
        }
        assert!(full >= 7, "expected most pushes refused, got {full}");
        // Another flow's queue is untouched by the stuck one.
        let (other, mut other_rx) = FlowQueue::new(2);
        assert_eq!(other.push(Bytes::from_static(b"y")), Push::Queued);
        assert_eq!(other_rx.recv().await.unwrap().as_ref(), b"y");
        writer.abort();
    }
}
