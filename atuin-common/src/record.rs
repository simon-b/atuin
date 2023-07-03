use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;
use uuid::Uuid;

pub type Diff = Vec<(Uuid, String, Uuid)>;

/// A single record stored inside of our local database
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TypedBuilder)]
pub struct Record {
    /// a unique ID
    #[builder(default = crate::utils::uuid_v7())]
    pub id: Uuid,

    /// The unique ID of the host.
    // TODO(ellie): Optimize the storage here. We use a bunch of IDs, and currently store
    // as strings. I would rather avoid normalization, so store as UUID binary instead of
    // encoding to a string and wasting much more storage.
    pub host: Uuid,

    /// The ID of the parent entry
    // A store is technically just a double linked list
    // We can do some cheating with the timestamps, but should not rely upon them.
    // Clocks are tricksy.
    #[builder(default)]
    pub parent: Option<Uuid>,

    /// The creation time in nanoseconds since unix epoch
    #[builder(default = chrono::Utc::now().timestamp_nanos() as u64)]
    pub timestamp: u64,

    /// The version the data in the entry conforms to
    // However we want to track versions for this tag, eg v2
    pub version: String,

    /// The type of data we are storing here. Eg, "history"
    pub tag: String,

    /// Some data. This can be anything you wish to store. Use the tag field to know how to handle it.
    pub data: Vec<u8>,
}

impl Record {
    pub fn new_child(&self, data: Vec<u8>) -> Record {
        Record::builder()
            .host(self.host.clone())
            .version(self.version.clone())
            .parent(Some(self.id.clone()))
            .tag(self.tag.clone())
            .data(data)
            .build()
    }
}

/// An index representing the current state of the record stores
/// This can be both remote, or local, and compared in either direction
#[derive(Debug, Serialize, Deserialize)]
pub struct RecordIndex {
    // A map of host -> tag -> tail
    pub hosts: HashMap<Uuid, HashMap<String, Uuid>>,
}

impl Default for RecordIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Vec<(Uuid, String, Uuid)>> for RecordIndex {
    fn from(f: Vec<(Uuid, String, Uuid)>) -> RecordIndex {
        let mut record_index = RecordIndex::new();

        for row in f {
            record_index.set_raw(row.0, row.1, row.2);
        }
        record_index
    }
}

impl RecordIndex {
    pub fn new() -> RecordIndex {
        RecordIndex {
            hosts: HashMap::new(),
        }
    }

    /// Insert a new tail record into the store
    pub fn set(&mut self, tail: Record) {
        self.set_raw(tail.host, tail.tag, tail.id)
    }

    pub fn set_raw(&mut self, host: Uuid, tag: String, tail: Uuid) {
        self.hosts.entry(host).or_default().insert(tag, tail);
    }

    pub fn get(&self, host: Uuid, tag: String) -> Option<Uuid> {
        self.hosts.get(&host).and_then(|v| v.get(&tag)).cloned()
    }

    /// Diff this index with another, likely remote index.
    /// The two diffs can then be reconciled, and the optimal change set calculated
    /// Returns a tuple, with (host, tag, Option(OTHER))
    /// OTHER is set to the value of the tail on the other machine. For example, if the
    /// other machine has a different tail, it will be the differing tail. This is useful to
    /// check if the other index is ahead of us, or behind.
    /// If the other index does not have the (host, tag) pair, then the other value will be None.
    pub fn diff(&self, other: &Self) -> Diff {
        let mut ret = Vec::new();

        // First, we check if other has everything that self has
        for (host, tag_map) in self.hosts.iter() {
            for (tag, tail) in tag_map.iter() {
                match other.get(host.clone(), tag.clone()) {
                    // The other store is all up to date! No diff.
                    Some(t) if t.eq(tail) => continue,

                    // The other store does exist, but it is either ahead or behind us. A diff regardless
                    Some(t) => ret.push((host.clone(), tag.clone(), t)),

                    // The other store does not exist :O
                    None => ret.push((host.clone(), tag.clone(), tail.clone())),
                };
            }
        }

        // At this point, there is a single case we have not yet considered.
        // If the other store knows of a tag that we are not yet aware of, then the diff will be missed

        // account for that!
        for (host, tag_map) in other.hosts.iter() {
            for (tag, tail) in tag_map.iter() {
                match self.get(host.clone(), tag.clone()) {
                    // If we have this host/tag combo, the comparison and diff will have already happened above
                    Some(_) => continue,

                    None => ret.push((host.clone(), tag.clone(), tail.clone())),
                };
            }
        }

        ret.sort();
        ret
    }
}

#[cfg(test)]
mod tests {
    use super::{Record, RecordIndex};
    use pretty_assertions::{assert_eq, assert_ne};
    use uuid::Uuid;

    fn test_record() -> Record {
        Record::builder()
            .host(crate::utils::uuid_v7())
            .version("v1".into())
            .tag(crate::utils::uuid_v7().simple().to_string())
            .data(vec![0, 1, 2, 3])
            .build()
    }

    #[test]
    fn record_index() {
        let mut index = RecordIndex::new();
        let record = test_record();

        index.set(record.clone());

        let tail = index.get(record.host, record.tag);

        assert_eq!(
            record.id,
            tail.expect("tail not in store"),
            "tail in store did not match"
        );
    }

    #[test]
    fn record_index_overwrite() {
        let mut index = RecordIndex::new();
        let record = test_record();
        let child = record.new_child(vec![1, 2, 3]);

        index.set(record.clone());
        index.set(child.clone());

        let tail = index.get(record.host, record.tag);

        assert_eq!(
            child.id,
            tail.expect("tail not in store"),
            "tail in store did not match"
        );
    }

    #[test]
    fn record_index_no_diff() {
        // Here, they both have the same version and should have no diff

        let mut index1 = RecordIndex::new();
        let mut index2 = RecordIndex::new();

        let record1 = test_record();

        index1.set(record1.clone());
        index2.set(record1);

        let diff = index1.diff(&index2);

        assert_eq!(0, diff.len(), "expected empty diff");
    }

    #[test]
    fn record_index_single_diff() {
        // Here, they both have the same stores, but one is ahead by a single record

        let mut index1 = RecordIndex::new();
        let mut index2 = RecordIndex::new();

        let record1 = test_record();
        let record2 = record1.new_child(vec![1, 2, 3]);

        index1.set(record1);
        index2.set(record2.clone());

        let diff = index1.diff(&index2);

        assert_eq!(1, diff.len(), "expected single diff");
        assert_eq!(diff[0], (record2.host, record2.tag, record2.id));
    }

    #[test]
    fn record_index_multi_diff() {
        // A much more complex case, with a bunch more checks
        let mut index1 = RecordIndex::new();
        let mut index2 = RecordIndex::new();

        let store1record1 = test_record();
        let store1record2 = store1record1.new_child(vec![1, 2, 3]);

        let store2record1 = test_record();
        let store2record2 = store2record1.new_child(vec![1, 2, 3]);

        let store3record1 = test_record();

        let store4record1 = test_record();

        // index1 only knows about the first two entries of the first two stores
        index1.set(store1record1);
        index1.set(store2record1);

        // index2 is fully up to date with the first two stores, and knows of a third
        index2.set(store1record2);
        index2.set(store2record2);
        index2.set(store3record1);

        // index1 knows of a 4th store
        index1.set(store4record1);

        let diff1 = index1.diff(&index2);
        let diff2 = index2.diff(&index1);

        // both diffs the same length
        assert_eq!(4, diff1.len());
        assert_eq!(4, diff2.len());

        // both diffs should be ALMOST the same. They will agree on which hosts and tags
        // require updating, but the "other" value will not be the same.
        let smol_diff_1: Vec<(Uuid, String)> =
            diff1.iter().map(|v| (v.0.clone(), v.1.clone())).collect();
        let smol_diff_2: Vec<(Uuid, String)> =
            diff1.iter().map(|v| (v.0.clone(), v.1.clone())).collect();

        assert_eq!(smol_diff_1, smol_diff_2);

        // diffing with yourself = no diff
        assert_eq!(index1.diff(&index1).len(), 0);
        assert_eq!(index2.diff(&index2).len(), 0);
    }
}
