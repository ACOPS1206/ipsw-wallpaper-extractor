Source: crates.io apfs 0.4.0 (Dil4rd/dpp), MIT; original LICENSE retained.

Only production change: `btree_scan_node` skips internal children entirely
before a requested range and stops at separators past the range. Include the
predecessor child because its tail may still match the range. The existing
comparator contract and object checksum verification are unchanged.

Regression tests cover a 100-leaf tree, every point/range boundary, predecessor
matches and misses beyond the last leaf. Upstream unit tests are also retained.
The optional benchmark and external fixture harnesses are omitted from this copy.
