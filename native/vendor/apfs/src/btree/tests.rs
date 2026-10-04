//! Unit tests for the parent module, split out of `btree.rs` for size.

use super::*;
use crate::fletcher;
use std::io::Cursor;

const BLOCK_SIZE: u32 = 4096;

/// Build a single-entry root leaf node in a checksummed 4 KiB block.
///
/// Layout: object header (32) | node header (24) | TOC (8) | key area |
/// ... | value | BTreeInfo (40, root only). One variable-size entry:
/// key = 8-byte LE u64, value = 8-byte LE u64.
fn synthetic_root_leaf(key: u64, value: u64) -> Vec<u8> {
    let mut block = vec![0u8; BLOCK_SIZE as usize];

    // Object header: checksum filled last; oid 1, xid 1, type BTREE, subtype 0.
    block[8..16].copy_from_slice(&1u64.to_le_bytes());
    block[16..24].copy_from_slice(&1u64.to_le_bytes());
    block[24..28].copy_from_slice(&object::OBJECT_TYPE_BTREE.to_le_bytes());

    // Node header at 32: root+leaf, level 0, one key, TOC space 8 bytes.
    let nh = ObjectHeader::SIZE;
    block[nh..nh + 2].copy_from_slice(&(BTNODE_ROOT | BTNODE_LEAF).to_le_bytes());
    block[nh + 4..nh + 8].copy_from_slice(&1u32.to_le_bytes());
    block[nh + 10..nh + 12].copy_from_slice(&8u16.to_le_bytes());

    // TOC at 56: key_off 0, key_len 8, val_off 8, val_len 8.
    let toc = nh + BTreeNodeHeader::SIZE;
    block[toc..toc + 2].copy_from_slice(&0u16.to_le_bytes());
    block[toc + 2..toc + 4].copy_from_slice(&8u16.to_le_bytes());
    block[toc + 4..toc + 6].copy_from_slice(&8u16.to_le_bytes());
    block[toc + 6..toc + 8].copy_from_slice(&8u16.to_le_bytes());

    // Key area at 64.
    let key_area = toc + 8;
    block[key_area..key_area + 8].copy_from_slice(&key.to_le_bytes());

    // Value grows down from val_area_end = block end minus BTreeInfo.
    let val_area_end = BLOCK_SIZE as usize - BTreeInfo::SIZE;
    block[val_area_end - 8..val_area_end].copy_from_slice(&value.to_le_bytes());

    // BTreeInfo: node_size, one key, one node; sizes 0 = variable.
    block[val_area_end + 4..val_area_end + 8].copy_from_slice(&BLOCK_SIZE.to_le_bytes());
    block[val_area_end + 24..val_area_end + 32].copy_from_slice(&1u64.to_le_bytes());
    block[val_area_end + 32..val_area_end + 40].copy_from_slice(&1u64.to_le_bytes());

    let checksum = fletcher::fletcher64(&block[8..]);
    block[..8].copy_from_slice(&checksum.to_le_bytes());
    block
}

fn compare_to(search: u64) -> impl Fn(&[u8]) -> Result<std::cmp::Ordering> {
    move |key: &[u8]| {
        let k = u64::from_le_bytes(key.try_into().expect("8-byte key"));
        Ok(k.cmp(&search))
    }
}

#[test]
fn lookup_reads_a_checksummed_node() {
    let block = synthetic_root_leaf(42, 7);
    let mut reader = Cursor::new(block);
    let found = btree_lookup(&mut reader, 0, BLOCK_SIZE, 0, 0, &compare_to(42), None)
        .expect("lookup on a valid node");
    assert_eq!(found.as_deref(), Some(&7u64.to_le_bytes()[..]));
}

#[test]
fn lookup_rejects_a_corrupt_node() {
    let mut block = synthetic_root_leaf(42, 7);
    block[100] ^= 0xFF;
    let mut reader = Cursor::new(block);
    let err = btree_lookup(&mut reader, 0, BLOCK_SIZE, 0, 0, &compare_to(42), None)
        .expect_err("corrupt node must not be traversed");
    assert!(matches!(err, ApfsError::InvalidChecksum), "{err:?}");
}

#[test]
fn comparator_errors_propagate_instead_of_reading_as_a_miss() {
    let block = synthetic_root_leaf(42, 7);
    let mut reader = Cursor::new(block);
    let failing = |_key: &[u8]| -> Result<std::cmp::Ordering> {
        Err(ApfsError::CorruptedData("undecodable key".into()))
    };
    let err = btree_lookup(&mut reader, 0, BLOCK_SIZE, 0, 0, &failing, None)
        .expect_err("an undecodable key must fail the lookup, not report a miss");
    assert!(matches!(err, ApfsError::CorruptedData(_)), "{err:?}");
}

#[test]
fn scan_rejects_a_corrupt_node() {
    let mut block = synthetic_root_leaf(42, 7);
    block[100] ^= 0xFF;
    let mut reader = Cursor::new(block);
    let err = btree_scan(&mut reader, 0, BLOCK_SIZE, 0, 0, &compare_to(42), None)
        .expect_err("corrupt node must not be scanned");
    assert!(matches!(err, ApfsError::InvalidChecksum), "{err:?}");
}

// Regression: a scan near the end of a large catalog must not read its prefix.
fn synthetic_node(entries: &[(u64, u64)], root: bool, leaf: bool) -> Vec<u8> {
    let mut block = vec![0; BLOCK_SIZE as usize];
    block[8..16].copy_from_slice(&1u64.to_le_bytes());
    block[16..24].copy_from_slice(&1u64.to_le_bytes());
    block[24..28].copy_from_slice(&object::OBJECT_TYPE_BTREE.to_le_bytes());
    let flags = if root { BTNODE_ROOT } else { 0 } | if leaf { BTNODE_LEAF } else { 0 };
    block[32..34].copy_from_slice(&flags.to_le_bytes());
    block[34..36].copy_from_slice(&(if leaf { 0u16 } else { 1u16 }).to_le_bytes());
    block[36..40].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    block[42..44].copy_from_slice(&((entries.len() * 8) as u16).to_le_bytes());
    let key_area = 56 + entries.len() * 8;
    let val_end = BLOCK_SIZE as usize - if root { BTreeInfo::SIZE } else { 0 };
    for (i, (key, value)) in entries.iter().enumerate() {
        let toc = 56 + i * 8;
        block[toc..toc + 2].copy_from_slice(&((i * 8) as u16).to_le_bytes());
        block[toc + 2..toc + 4].copy_from_slice(&8u16.to_le_bytes());
        block[toc + 4..toc + 6].copy_from_slice(&((i * 8 + 8) as u16).to_le_bytes());
        block[toc + 6..toc + 8].copy_from_slice(&8u16.to_le_bytes());
        block[key_area + i * 8..key_area + i * 8 + 8].copy_from_slice(&key.to_le_bytes());
        block[val_end - i * 8 - 8..val_end - i * 8].copy_from_slice(&value.to_le_bytes());
    }
    if root {
        block[val_end + 4..val_end + 8].copy_from_slice(&BLOCK_SIZE.to_le_bytes());
    }
    let checksum = fletcher::fletcher64(&block[8..]);
    block[..8].copy_from_slice(&checksum.to_le_bytes());
    block
}
struct CountedTree {
    inner: Cursor<Vec<u8>>,
    reads: usize,
}
impl Read for CountedTree {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.reads += 1;
        self.inner.read(buffer)
    }
}
impl Seek for CountedTree {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}
fn wide_tree() -> CountedTree {
    let separators: Vec<_> = (0..100).map(|i| (i * 7, i + 1)).collect();
    let mut data = synthetic_node(&separators, true, false);
    for i in 0..100 {
        let values: Vec<_> = (i * 7..i * 7 + 7).map(|key| (key, key + 1000)).collect();
        data.extend(synthetic_node(&values, false, true));
    }
    CountedTree {
        inner: Cursor::new(data),
        reads: 0,
    }
}
#[test]
fn scan_near_catalog_end_reads_only_target_subtree() {
    let mut tree = wide_tree();
    let result = btree_scan(&mut tree, 0, BLOCK_SIZE, 0, 0, &compare_to(695), None).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(
        u64::from_le_bytes(result[0].1.clone().try_into().unwrap()),
        1695
    );
    assert_eq!(tree.reads, 2, "scan traversed unrelated preceding leaves");
}
#[test]
fn range_spanning_children_keeps_predecessor_tail() {
    let mut tree = wide_tree();
    let compare = |key: &[u8]| Ok((u64::from_le_bytes(key.try_into().unwrap()) / 10).cmp(&5));
    let result = btree_scan(&mut tree, 0, BLOCK_SIZE, 0, 0, &compare, None).unwrap();
    let keys: Vec<_> = result
        .iter()
        .map(|(key, _)| u64::from_le_bytes(key.clone().try_into().unwrap()))
        .collect();
    assert_eq!(keys, (50..60).collect::<Vec<_>>());
    assert_eq!(
        tree.reads, 3,
        "only the root and two overlapping leaves are needed"
    );
}
#[test]
fn missing_high_key_does_not_scan_entire_catalog() {
    let mut tree = wide_tree();
    assert!(
        btree_scan(&mut tree, 0, BLOCK_SIZE, 0, 0, &compare_to(1000), None)
            .unwrap()
            .is_empty()
    );
    assert_eq!(tree.reads, 2);
}
#[test]
fn optimized_scan_matches_expected_results_for_all_boundaries() {
    for search in 0..=701 {
        let mut tree = wide_tree();
        let result = btree_scan(&mut tree, 0, BLOCK_SIZE, 0, 0, &compare_to(search), None).unwrap();
        assert_eq!(result.len(), usize::from(search < 700), "key {search}");
        if search < 700 {
            assert_eq!(
                u64::from_le_bytes(result[0].0.clone().try_into().unwrap()),
                search
            );
        }
    }
    for group in 0..=71 {
        let mut tree = wide_tree();
        let compare =
            |key: &[u8]| Ok((u64::from_le_bytes(key.try_into().unwrap()) / 10).cmp(&group));
        let result = btree_scan(&mut tree, 0, BLOCK_SIZE, 0, 0, &compare, None).unwrap();
        let keys: Vec<_> = result
            .iter()
            .map(|(key, _)| u64::from_le_bytes(key.clone().try_into().unwrap()))
            .collect();
        assert_eq!(
            keys,
            (0..700).filter(|key| key / 10 == group).collect::<Vec<_>>()
        );
    }
}
