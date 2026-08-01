use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

const SEQUENCE_BITS: u64 = 12;
const NODE_BITS: u64 = 10;

const SEQUENCE_MASK: u64 = (1 << SEQUENCE_BITS) - 1;
const NODE_MASK: u64 = (1 << NODE_BITS) - 1;

const TIMESTAMP_SHIFT: u64 = SEQUENCE_BITS + NODE_BITS;

const EPOCH_SECONDS: u64 = 1_704_067_200;

#[derive(Debug)]
pub struct IdGenerator {
    epoch: SystemTime,
    node: u64,
    last: AtomicU64,
}

impl IdGenerator {
    pub fn new(node: u64) -> Self {
        Self {
            epoch: SystemTime::UNIX_EPOCH + Duration::from_secs(EPOCH_SECONDS),
            node: node & NODE_MASK,
            last: AtomicU64::new(0),
        }
    }

    pub fn node(&self) -> u64 {
        self.node
    }

    pub fn generate(&self) -> u64 {
        let now_ms = self
            .epoch
            .elapsed()
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);

        let mut previous = self.last.load(Ordering::Relaxed);
        loop {
            let previous_ms = previous >> SEQUENCE_BITS;
            let previous_seq = previous & SEQUENCE_MASK;

            let (next_ms, next_seq) = if now_ms > previous_ms {
                (now_ms, 0)
            } else if previous_seq < SEQUENCE_MASK {
                (previous_ms, previous_seq + 1)
            } else {
                (previous_ms + 1, 0)
            };

            let candidate = (next_ms << SEQUENCE_BITS) | next_seq;
            match self.last.compare_exchange_weak(
                previous,
                candidate,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return (next_ms << TIMESTAMP_SHIFT) | (self.node << SEQUENCE_BITS) | next_seq;
                }
                Err(observed) => previous = observed,
            }
        }
    }

    pub fn timestamp_of(id: u64) -> u64 {
        ((id >> TIMESTAMP_SHIFT) / 1_000) + EPOCH_SECONDS
    }
}

impl Default for IdGenerator {
    fn default() -> Self {
        Self::new(0)
    }
}

#[derive(Debug)]
pub struct DocumentIdAllocator {
    next: AtomicU32,
}

impl DocumentIdAllocator {
    pub fn new() -> Self {
        Self {
            next: AtomicU32::new(0),
        }
    }

    pub fn from_highest(highest: Option<u32>) -> Self {
        let start = highest.map(|id| id.saturating_add(1)).unwrap_or(0);
        Self {
            next: AtomicU32::new(start),
        }
    }

    pub fn next(&self) -> Option<u32> {
        let mut current = self.next.load(Ordering::Relaxed);
        loop {
            if current == u32::MAX {
                return None;
            }
            match self.next.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(current),
                Err(observed) => current = observed,
            }
        }
    }

    pub fn peek(&self) -> u32 {
        self.next.load(Ordering::Relaxed)
    }
}

impl Default for DocumentIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[test]
    fn generated_ids_are_unique_and_strictly_increasing_in_a_burst() {
        let generator = IdGenerator::new(7);
        let mut seen = HashSet::new();
        let mut previous = 0u64;

        for _ in 0..50_000 {
            let id = generator.generate();
            assert!(seen.insert(id), "duplicate id produced: {id}");
            assert!(id > previous, "id did not increase: {id} <= {previous}");
            previous = id;
        }
    }

    #[test]
    fn node_id_is_masked_into_range_and_packed() {
        let node = NODE_MASK + 5;
        let generator = IdGenerator::new(node);
        assert_eq!(generator.node(), node & NODE_MASK);

        let id = generator.generate();
        let packed_node = (id >> SEQUENCE_BITS) & NODE_MASK;
        assert_eq!(packed_node, node & NODE_MASK);
    }

    #[test]
    fn timestamp_round_trips_to_a_sane_wall_clock() {
        let generator = IdGenerator::new(0);
        let id = generator.generate();
        let seconds = IdGenerator::timestamp_of(id);

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(seconds <= now);
        assert!(seconds >= EPOCH_SECONDS);
        assert!(now - seconds < 60);
    }

    #[test]
    fn generation_is_unique_across_threads_on_one_instance() {
        let generator = Arc::new(IdGenerator::new(1));
        let mut handles = Vec::new();

        for _ in 0..4 {
            let generator = Arc::clone(&generator);
            handles.push(std::thread::spawn(move || {
                (0..2_000).map(|_| generator.generate()).collect::<Vec<_>>()
            }));
        }

        let mut seen = HashSet::new();
        for handle in handles {
            for id in handle.join().unwrap() {
                assert!(seen.insert(id), "duplicate id across threads: {id}");
            }
        }
        assert_eq!(seen.len(), 8_000);
    }

    #[test]
    fn document_allocator_starts_at_zero() {
        let allocator = DocumentIdAllocator::new();
        assert_eq!(allocator.peek(), 0);
        assert_eq!(allocator.next(), Some(0));
        assert_eq!(allocator.next(), Some(1));
        assert_eq!(allocator.next(), Some(2));
    }

    #[test]
    fn document_allocator_resumes_past_the_highest_existing_id() {
        let fresh = DocumentIdAllocator::from_highest(None);
        assert_eq!(fresh.next(), Some(0));

        let resumed = DocumentIdAllocator::from_highest(Some(41));
        assert_eq!(resumed.peek(), 42);
        assert_eq!(resumed.next(), Some(42));
        assert_eq!(resumed.next(), Some(43));
    }

    #[test]
    fn document_ids_are_monotonic_and_unique_under_contention() {
        let allocator = Arc::new(DocumentIdAllocator::new());
        let mut handles = Vec::new();

        for _ in 0..4 {
            let allocator = Arc::clone(&allocator);
            handles.push(std::thread::spawn(move || {
                (0..1_000)
                    .map(|_| allocator.next().unwrap())
                    .collect::<Vec<_>>()
            }));
        }

        let mut seen = HashSet::new();
        for handle in handles {
            for id in handle.join().unwrap() {
                assert!(seen.insert(id), "duplicate document id: {id}");
            }
        }
        assert_eq!(seen.len(), 4_000);
    }
}
