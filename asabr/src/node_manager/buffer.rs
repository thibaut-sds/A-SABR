extern crate alloc;
use crate::parse_transparent;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::convert::TryFrom;

use crate::{
    bundle::Bundle,
    errors::ASABRError,
    node_manager::NodeManager,
    types::{Date, NodeID, TimeInterval, Volume},
};

/// Storage backend for BufferManager
pub trait BufferStorage {
    fn new() -> Self;
    fn check_capacity(&self, start: Date, end: Date, size: Volume, capacity: Volume) -> bool;
    fn commit(&mut self, start: Date, end: Date, size: Volume);

    #[cfg(test)]
    fn from_entries(entries: &[(Date, Volume)]) -> Self;
    #[cfg(test)]
    fn to_entries(&self) -> Vec<(Date, Volume)>;
}

#[derive(Debug, Clone)]
pub struct BTreeStorage {
    memory: BTreeMap<Date, Volume>,
}

impl BufferStorage for BTreeStorage {
    fn new() -> Self {
        let mut memory = BTreeMap::new();
        // Sentinel at t=0 so every date has a preceding entry
        memory.insert(0, 0);
        Self { memory }
    }

    fn check_capacity(&self, start: Date, end: Date, size: Volume, capacity: Volume) -> bool {
        if end < start {
            return false;
        }
        if end == start {
            return true;
        }

        let current_vol = self
            .memory
            .range(..=start)
            .next_back()
            .map(|(_, &v)| v)
            .unwrap_or(0);

        if current_vol + size > capacity {
            return false;
        }

        for (_, &vol) in self.memory.range((start + 1)..end) {
            if vol + size > capacity {
                return false;
            }
        }

        true
    }

    fn commit(&mut self, start: Date, end: Date, size: Volume) {
        if end <= start {
            return;
        }

        let vol_at_start = self
            .memory
            .range(..=start)
            .next_back()
            .map(|(_, &v)| v)
            .unwrap_or(0);
        self.memory.entry(start).or_insert(vol_at_start);

        let vol_at_release = self
            .memory
            .range(..=end)
            .next_back()
            .map(|(_, &v)| v)
            .unwrap_or(0);
        self.memory.entry(end).or_insert(vol_at_release);

        for (_, vol) in self.memory.range_mut(start..end) {
            *vol += size;
        }
    }

    #[cfg(test)]
    fn from_entries(entries: &[(Date, Volume)]) -> Self {
        let mut memory = BTreeMap::new();
        for &(k, v) in entries {
            memory.insert(k, v);
        }
        Self { memory }
    }

    #[cfg(test)]
    fn to_entries(&self) -> Vec<(Date, Volume)> {
        self.memory.iter().map(|(&k, &v)| (k, v)).collect()
    }
}

#[derive(Debug, Clone)]
pub struct VecStorage {
    /// Sorted by date, each entry holds the active volume from its date until the next entry
    memory: Vec<(Date, Volume)>,
}

impl BufferStorage for VecStorage {
    fn new() -> Self {
        Self {
            memory: vec![(0, 0)],
        }
    }

    fn check_capacity(&self, start: Date, end: Date, size: Volume, capacity: Volume) -> bool {
        if end < start {
            return false;
        }
        if end == start {
            return true;
        }

        // First entry with date > start, idx - 1 is the active volume at "start"
        let idx = self.memory.partition_point(|&(d, _)| d <= start);
        let current_vol = if idx > 0 { self.memory[idx - 1].1 } else { 0 };

        if current_vol + size > capacity {
            return false;
        }

        for &(d, vol) in &self.memory[idx..] {
            if d >= end {
                break;
            }
            if vol + size > capacity {
                return false;
            }
        }

        true
    }

    fn commit(&mut self, start: Date, end: Date, size: Volume) {
        if end <= start {
            return;
        }

        let start_idx = match self.memory.binary_search_by_key(&start, |&(d, _)| d) {
            Ok(idx) => idx,
            Err(idx) => {
                let vol_at_start = if idx > 0 { self.memory[idx - 1].1 } else { 0 };
                self.memory.insert(idx, (start, vol_at_start));
                idx
            }
        };

        // Update [start, end) in one pass and keep the pre-update volume for the "end" boundary
        let mut i = start_idx;
        let mut vol_before_end = 0;
        while i < self.memory.len() && self.memory[i].0 < end {
            vol_before_end = self.memory[i].1;
            self.memory[i].1 += size;
            i += 1;
        }

        if i == self.memory.len() || self.memory[i].0 > end {
            self.memory.insert(i, (end, vol_before_end));
        }
    }

    #[cfg(test)]
    fn from_entries(entries: &[(Date, Volume)]) -> Self {
        Self {
            memory: entries.to_vec(),
        }
    }

    #[cfg(test)]
    fn to_entries(&self) -> Vec<(Date, Volume)> {
        self.memory.clone()
    }
}

/// Manages the temporal memory buffer of a node
#[derive(Debug, Clone)]
pub struct BufferManager<S: BufferStorage = VecStorage> {
    storage: S,
    capacity: Volume,
}

pub type VecBufferManager = BufferManager<VecStorage>;
pub type BTreeBufferManager = BufferManager<BTreeStorage>;

impl<S: BufferStorage> BufferManager<S> {
    pub fn new(capacity: Volume) -> Self {
        Self {
            storage: S::new(),
            capacity,
        }
    }

    fn check_capacity(&self, start: Date, end: Date, size: Volume) -> bool {
        self.storage.check_capacity(start, end, size, self.capacity)
    }
}

impl<S: BufferStorage> NodeManager for BufferManager<S> {
    fn accept(&self, bundle: &Bundle, reception: TimeInterval, _sender: NodeID) -> bool {
        self.check_capacity(reception.start, reception.end, bundle.size)
    }

    fn process_delay(
        &self,
        _bundle: &Bundle,
        reception: TimeInterval,
        _sender: NodeID,
        _next_vertex: NodeID,
    ) -> Date {
        reception.end
    }

    fn dry_run_retention(
        &self,
        bundle: &Bundle,
        reception: TimeInterval,
        _sender: NodeID,
        transmission: TimeInterval,
        _next_vertex: NodeID,
    ) -> bool {
        self.check_capacity(reception.start, transmission.end, bundle.size)
    }

    fn dry_run_multi(
        &self,
        bundle: &Bundle,
        reception: TimeInterval,
        _sender: NodeID,
        transmissions: &[(TimeInterval, NodeID)],
    ) -> Option<usize> {
        let mut release = reception.end;
        for (interval, _) in transmissions {
            if interval.end > release {
                release = interval.end;
            }
        }

        if self.check_capacity(reception.start, release, bundle.size) {
            Some(transmissions.len())
        } else {
            None
        }
    }

    fn commit(
        &mut self,
        bundle: &Bundle,
        reception: TimeInterval,
        _sender: NodeID,
        transmissions: &[(TimeInterval, NodeID)],
    ) -> Result<(), ASABRError> {
        let mut release = reception.end;
        for (interval, _) in transmissions {
            if interval.end > release {
                release = interval.end;
            }
        }

        self.storage.commit(reception.start, release, bundle.size);
        Ok(())
    }
}

impl<S: BufferStorage> TryFrom<Volume> for BufferManager<S> {
    type Error = ();

    fn try_from(capacity: Volume) -> Result<Self, Self::Error> {
        Ok(BufferManager::new(capacity))
    }
}

parse_transparent!(VecBufferManager, Volume);
parse_transparent!(BTreeBufferManager, Volume);

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(start: Date, end: Date) -> TimeInterval {
        TimeInterval { start, end }
    }

    fn bundle(size: Volume) -> Bundle {
        Bundle {
            priority: 0,
            size,
            expiration: 1000,
        }
    }

    fn sender() -> NodeID {
        1usize.into()
    }

    fn next() -> NodeID {
        2usize.into()
    }

    fn transmission(start: Date, end: Date, next_node: NodeID) -> (TimeInterval, NodeID) {
        (interval(start, end), next_node)
    }

    macro_rules! buffer_storage_tests {
        ($mod_name:ident, $storage:ty) => {
            mod $mod_name {
                use super::*;

                fn manager(capacity: Volume, memory: &[(Date, Volume)]) -> BufferManager<$storage> {
                    BufferManager {
                        storage: <$storage>::from_entries(memory),
                        capacity,
                    }
                }

                #[test]
                fn check_capacity_accepts_empty_memory_and_zero_length_intervals() {
                    let manager = BufferManager::<$storage>::new(10);
                    assert!(manager.check_capacity(0, 10, 10));
                    assert!(manager.check_capacity(5, 5, 1000));
                    assert!(!manager.check_capacity(6, 5, 1));
                }

                #[test]
                fn check_capacity_handles_one_tuple_boundaries() {
                    let manager = manager(10, &[(0, 0), (5, 4)]);
                    assert!(manager.check_capacity(0, 5, 10));
                    assert!(manager.check_capacity(0, 6, 6));
                    assert!(!manager.check_capacity(0, 6, 7));
                    assert!(manager.check_capacity(5, 10, 6));
                    assert!(!manager.check_capacity(5, 10, 7));
                    assert!(manager.check_capacity(20, 30, 6));
                }

                #[test]
                fn check_capacity_handles_several_tuples_and_start_positions() {
                    let manager = manager(10, &[(0, 0), (10, 3), (20, 8), (30, 1)]);
                    assert!(manager.check_capacity(0, 10, 10));
                    assert!(manager.check_capacity(10, 20, 7));
                    assert!(!manager.check_capacity(10, 20, 8));
                    assert!(manager.check_capacity(20, 30, 2));
                    assert!(!manager.check_capacity(20, 30, 3));
                    assert!(manager.check_capacity(15, 18, 7));
                    assert!(!manager.check_capacity(15, 25, 3));
                    assert!(manager.check_capacity(40, 50, 9));
                }

                #[test]
                fn check_capacity_handles_end_positions() {
                    let manager = manager(10, &[(0, 0), (10, 2), (20, 5), (30, 9)]);
                    assert!(manager.check_capacity(0, 9, 10));
                    assert!(manager.check_capacity(0, 10, 10));
                    assert!(manager.check_capacity(0, 25, 5));
                    assert!(!manager.check_capacity(0, 35, 2));
                    assert!(manager.check_capacity(20, 30, 5));
                    assert!(!manager.check_capacity(20, 31, 2));
                }

                #[test]
                fn check_capacity_counts_internal_tuples_and_capacity_edges() {
                    let manager = manager(10, &[(0, 0), (10, 3), (20, 6), (30, 4), (40, 1)]);
                    assert!(manager.check_capacity(11, 19, 7));
                    assert!(manager.check_capacity(11, 21, 4));
                    assert!(manager.check_capacity(11, 39, 4));
                    assert!(!manager.check_capacity(11, 39, 5));
                    assert!(manager.check_capacity(20, 30, 4));
                    assert!(!manager.check_capacity(20, 30, 5));
                }

                #[test]
                fn check_capacity_uses_current_volume_before_start() {
                    let manager = manager(10, &[(0, 0), (10, 2), (20, 9), (30, 1)]);
                    assert!(manager.check_capacity(25, 28, 1));
                    assert!(!manager.check_capacity(25, 28, 2));
                    assert!(manager.check_capacity(35, 45, 9));
                    assert!(!manager.check_capacity(35, 45, 10));
                }

                #[test]
                fn accept_matches_check_capacity_for_success_and_capacity_failures() {
                    let manager = manager(10, &[(0, 0), (10, 3), (20, 8), (30, 1)]);
                    let small = bundle(2);
                    let large = bundle(3);

                    for reception in [
                        interval(0, 5),
                        interval(10, 20),
                        interval(15, 25),
                        interval(20, 30),
                        interval(40, 50),
                    ] {
                        assert_eq!(
                            manager.accept(&small, reception, sender()),
                            manager.check_capacity(reception.start, reception.end, small.size)
                        );
                        assert_eq!(
                            manager.accept(&large, reception, sender()),
                            manager.check_capacity(reception.start, reception.end, large.size)
                        );
                    }
                }

                #[test]
                fn accept_matches_check_capacity_with_empty_memory() {
                    let manager = BufferManager::<$storage>::new(5);
                    let fits = bundle(5);
                    let too_large = bundle(6);

                    assert_eq!(
                        manager.accept(&fits, interval(0, 10), sender()),
                        manager.check_capacity(0, 10, fits.size)
                    );
                    assert_eq!(
                        manager.accept(&too_large, interval(0, 10), sender()),
                        manager.check_capacity(0, 10, too_large.size)
                    );
                }

                #[test]
                fn dry_run_retention_uses_reception_start_and_transmission_end() {
                    let manager = manager(10, &[(0, 0), (10, 1), (20, 9), (30, 1)]);
                    let b = bundle(1);
                    assert!(manager.dry_run_retention(
                        &b,
                        interval(5, 15),
                        sender(),
                        interval(100, 20),
                        next(),
                    ));
                    assert!(!manager.dry_run_retention(
                        &bundle(2),
                        interval(5, 15),
                        sender(),
                        interval(100, 21),
                        next(),
                    ));
                }

                #[test]
                fn dry_run_multi_handles_no_one_and_several_transmissions() {
                    let manager = manager(10, &[(0, 0), (10, 4), (20, 7), (30, 1)]);

                    assert_eq!(
                        manager.dry_run_multi(&bundle(6), interval(0, 10), sender(), &[]),
                        Some(0)
                    );
                    assert_eq!(
                        manager.dry_run_multi(
                            &bundle(3),
                            interval(10, 11),
                            sender(),
                            &[transmission(12, 20, next())]
                        ),
                        Some(1)
                    );
                    assert_eq!(
                        manager.dry_run_multi(
                            &bundle(3),
                            interval(10, 11),
                            sender(),
                            &[
                                transmission(18, 21, next()),
                                transmission(12, 25, 3usize.into()),
                                transmission(14, 19, 4usize.into()),
                            ]
                        ),
                        Some(3)
                    );
                }

                #[test]
                fn dry_run_multi_returns_none_when_capacity_is_insufficient() {
                    let manager = manager(10, &[(0, 0), (10, 4), (20, 8), (30, 1)]);
                    assert_eq!(
                        manager.dry_run_multi(
                            &bundle(3),
                            interval(10, 11),
                            sender(),
                            &[transmission(12, 25, next())]
                        ),
                        None
                    );
                }

                #[test]
                fn commit_updates_empty_memory_and_first_tuple() {
                    let mut manager = BufferManager::<$storage>::new(10);
                    manager
                        .commit(
                            &bundle(3),
                            interval(5, 7),
                            sender(),
                            &[transmission(10, 20, next())],
                        )
                        .unwrap();

                    let expected: Vec<(Date, Volume)> = vec![(0, 0), (5, 3), (20, 0)];
                    assert_eq!(manager.storage.to_entries(), expected);
                }

                #[test]
                fn commit_inserts_start_and_release_boundaries() {
                    let mut manager = manager(10, &[(0, 0), (10, 2), (20, 4), (30, 1)]);
                    manager
                        .commit(
                            &bundle(3),
                            interval(15, 18),
                            sender(),
                            &[transmission(22, 25, next())],
                        )
                        .unwrap();

                    let expected: Vec<(Date, Volume)> =
                        vec![(0, 0), (10, 2), (15, 5), (20, 7), (25, 4), (30, 1)];
                    assert_eq!(manager.storage.to_entries(), expected);
                }

                #[test]
                fn commit_reuses_existing_boundaries_without_duplicates() {
                    let mut manager = manager(10, &[(0, 0), (10, 2), (20, 4), (30, 1)]);
                    manager
                        .commit(
                            &bundle(3),
                            interval(10, 12),
                            sender(),
                            &[transmission(15, 20, next())],
                        )
                        .unwrap();

                    let expected: Vec<(Date, Volume)> = vec![(0, 0), (10, 5), (20, 4), (30, 1)];
                    assert_eq!(manager.storage.to_entries(), expected);
                }

                #[test]
                fn commit_handles_asymmetric_overlapping_bundles() {
                    let mut manager = BufferManager::<$storage>::new(1000);

                    manager
                        .commit(
                            &bundle(600),
                            interval(0, 10),
                            sender(),
                            &[transmission(60, 70, next())],
                        )
                        .unwrap();
                    manager
                        .commit(
                            &bundle(300),
                            interval(20, 30),
                            sender(),
                            &[transmission(80, 90, next())],
                        )
                        .unwrap();

                    let expected: Vec<(Date, Volume)> =
                        vec![(0, 600), (20, 900), (70, 300), (90, 0)];
                    assert_eq!(manager.storage.to_entries(), expected);

                    // 600 + 300 + 250 = 1150 > 1000 on [40, 60]
                    assert!(!manager.check_capacity(40, 60, 250));
                    // Fits once B1 is released at t=70
                    assert!(manager.check_capacity(70, 90, 700));
                }
            }
        };
    }

    buffer_storage_tests!(vec_storage, VecStorage);
    buffer_storage_tests!(btree_storage, BTreeStorage);
}
