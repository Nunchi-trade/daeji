# Storage and State Management

## Summary

Kora uses a layered storage architecture: `OverlayState<QmdbState>` wraps a persistent QMDB store with in-memory pending changes. State roots are computed using a deterministic transition root (keccak256 of parent root + sorted changes). The storage layer uses a generation-based key scheme to handle account selfdestruct/re-creation correctly.

---

## 1. OverlayState

**File:** `crates/storage/overlay/src/overlay.rs`

### Structure (Lines 8-12)

```rust
pub struct OverlayState<S> {
    base: S,                    // Underlying QMDB state
    changes: Arc<ChangeSet>,    // Pending changes layered on top
}
```

### Read Path — Layered Lookups

All reads check the overlay first, then fall back to the base:

#### Nonce/Balance (Lines 37-65)
```rust
fn nonce(&self, address: &Address) -> Result<u64, StateDbError> {
    if let Some(update) = changes.accounts.get(&address) {
        return Ok(update.nonce);  // From overlay
    }
    base.nonce(&address).await    // From QMDB
}
```

#### Storage with Selfdestruct Handling (Lines 101-124)
```rust
fn storage(&self, address: &Address, slot: &U256) -> Result<U256, StateDbError> {
    if let Some(update) = changes.accounts.get(&address) {
        if update.selfdestructed {
            return Ok(U256::ZERO);     // Selfdestruct clears all storage
        }
        if let Some(value) = update.storage.get(&slot) {
            return Ok(*value);          // Changed slot in overlay
        }
        if update.created {
            return Ok(U256::ZERO);     // Newly created accounts have no storage
        }
    }
    base.storage(&address, &slot).await  // From QMDB
}
```

#### Code Lookup (Lines 89-96)
```rust
fn code(&self, code_hash: &B256) -> Result<Bytes, StateDbError> {
    for update in changes.accounts.values() {
        if update.code_hash == code_hash && let Some(code) = &update.code {
            return Ok(Bytes::from(code.clone()));  // Code in overlay
        }
    }
    base.code(&code_hash).await  // Code in QMDB
}
```

**Note:** Code lookup iterates ALL accounts in the changeset to find matching code_hash — O(n) per lookup.

### Write Path — Merge and Commit (Lines 128-153)

```rust
fn commit(&self, changes: ChangeSet) -> Result<B256, StateDbError> {
    let mut merged = (*self.changes).clone();
    merged.merge(changes);
    base.commit(merged).await   // Commit merged changes to QMDB
}

fn compute_root(&self, changes: &ChangeSet) -> Result<B256, StateDbError> {
    let mut merged = (*self.changes).clone();
    merged.merge(changes.clone());
    base.compute_root(&merged).await  // Compute root without committing
}
```

### Concurrency Model

- `Arc<ChangeSet>` is **immutable after creation** — multiple readers can share
- No write contention on the overlay itself
- Writes go through `commit()` which creates a new merged changeset
- **No locking within OverlayState** — thread safety via immutable sharing

---

## 2. ChangeSet

**File:** `crates/storage/qmdb/src/changes.rs`

### Structure (Lines 9-12)

```rust
pub struct ChangeSet {
    pub accounts: BTreeMap<Address, AccountUpdate>,  // Sorted by address
}
```

### AccountUpdate (Lines 54-69)

```rust
pub struct AccountUpdate {
    pub created: bool,           // Account was created in this change
    pub selfdestructed: bool,    // Account was selfdestructed
    pub nonce: u64,              // Current nonce
    pub balance: U256,           // Current balance
    pub code_hash: B256,         // Code hash
    pub code: Option<Vec<u8>>,   // New code (if deployed)
    pub storage: BTreeMap<U256, U256>,  // Storage slot changes
}
```

### Merge Logic (Lines 71-99)

```rust
pub fn merge(&mut self, other: Self) {
    // If created: clear existing storage (fresh account)
    if created { self.storage.clear(); self.created = true; }
    // If selfdestructed: clear storage
    if selfdestructed { self.storage.clear(); }

    self.selfdestructed = selfdestructed;
    self.nonce = nonce;      // Overwrites (latest wins)
    self.balance = balance;  // Overwrites (latest wins)

    // Code update if hash changed or new code provided
    if self.code_hash != code_hash || code.is_some() {
        self.code = code;
    }
    self.code_hash = code_hash;

    // Merge storage changes (unless selfdestructed)
    if !selfdestructed {
        for (slot, value) in storage {
            self.storage.insert(slot, value);
        }
    }
}
```

**Key invariant:** BTreeMap iteration order is deterministic (sorted by key), ensuring all validators produce the same merged changesets.

---

## 3. State Root Computation

**File:** `crates/storage/qmdb/src/root.rs`

### Two-Root System

#### Root 1: Partition-based Root (Lines 15-23)

```rust
pub fn compute(accounts_root: B256, storage_root: B256, code_root: B256) -> B256 {
    let mut buf = Vec::with_capacity(KORA_ROOT_NAMESPACE.len() + 96);
    buf.extend_from_slice(KORA_ROOT_NAMESPACE);  // b"_KORA_QMDB_ROOT"
    buf.extend_from_slice(accounts_root.as_slice());
    buf.extend_from_slice(storage_root.as_slice());
    buf.extend_from_slice(code_root.as_slice());
    keccak256(buf)
}
```

Used for the physical QMDB storage layer. Not used in consensus.

#### Root 2: Consensus Transition Root (Lines 25-61) — USED IN CONSENSUS

```rust
pub fn transition(parent_root: B256, changes: &ChangeSet) -> B256 {
    if changes.is_empty() { return parent_root; }  // Empty blocks inherit parent

    let mut buf = Vec::new();
    buf.extend_from_slice(KORA_TRANSITION_ROOT_NAMESPACE);  // b"_KORA_STATE_TRANSITION_ROOT"
    buf.extend_from_slice(parent_root.as_slice());
    buf.extend_from_slice(&(changes.accounts.len() as u64).to_be_bytes());

    for (address, update) in &changes.accounts {  // BTreeMap = sorted iteration
        buf.extend_from_slice(address.as_slice());
        buf.push(u8::from(update.created));
        buf.push(u8::from(update.selfdestructed));
        buf.extend_from_slice(&update.nonce.to_be_bytes());
        buf.extend_from_slice(&update.balance.to_be_bytes::<32>());
        buf.extend_from_slice(update.code_hash.as_slice());

        // Code presence + content
        match &update.code {
            Some(code) => { buf.push(1); buf.extend(code.len().to_be_bytes()); buf.extend(code); }
            None => buf.push(0),
        }

        // Storage changes (also sorted via BTreeMap)
        buf.extend_from_slice(&(update.storage.len() as u64).to_be_bytes());
        for (slot, value) in &update.storage {
            buf.extend_from_slice(&slot.to_be_bytes::<32>());
            buf.extend_from_slice(&value.to_be_bytes::<32>());
        }
    }

    keccak256(buf)
}
```

**Determinism guarantees:**
- Parent root is agreed upon by consensus
- Changes are in `BTreeMap` (sorted by address)
- Storage changes are in `BTreeMap` (sorted by slot)
- All field serialization is big-endian fixed-width
- Empty blocks inherit parent root (no hash computation)

**Used at:** `crates/node/ledger/src/lib.rs:276-287`

```rust
pub async fn compute_root_from_store(&self, parent: ConsensusDigest, changes: QmdbChangeSet) -> LedgerResult<StateRoot> {
    let parent_root = inner.snapshots.get(&parent)?.state_root;
    Ok(StateRoot(QmdbStateRoot::transition(parent_root.0, &changes)))
}
```

---

## 4. QMDB Storage Layer

**File:** `crates/storage/qmdb/src/store.rs`

### Three-Partition Architecture (Lines 14-29)

```rust
pub struct Stores<A, S, C> {
    pub accounts: A,   // Account data (nonce, balance, code_hash, generation)
    pub storage: S,    // Storage slots (address + generation + slot → value)
    pub code: C,       // Contract bytecode (code_hash → bytes)
}
```

### Account Encoding (encoding.rs:52-59)

```rust
// 80-byte fixed encoding:
// [0..8]   nonce (u64 big-endian)
// [8..40]  balance (U256 big-endian)
// [40..72] code_hash (B256)
// [72..80] generation (u64 big-endian)
```

### Generation-Based Storage Keys (encoding.rs:16-39)

```rust
pub struct StorageKey {
    pub address: Address,      // 20 bytes
    pub generation: u64,       // 8 bytes — increments on selfdestruct/recreate
    pub slot: U256,            // 32 bytes
}
// Total key: 60 bytes
```

**Purpose:** When an account is selfdestructed and recreated, the generation increments. This effectively invalidates all old storage without needing to delete individual slots. New storage writes use the new generation, and old storage is orphaned.

### Batch Building (store.rs:138-188)

```rust
pub async fn build_batches(&self, changes: &ChangeSet) -> Result<StoreBatches, QmdbError> {
    for (address, update) in &changes.accounts {
        let current_gen = /* read from store */;

        // Increment generation on create or selfdestruct
        let new_gen = if update.created || update.selfdestructed {
            current_gen.saturating_add(1)
        } else {
            current_gen
        };

        // Account write
        if update.selfdestructed {
            batches.accounts.push((*address, None));  // Delete
        } else {
            let encoded = AccountEncoding::encode(nonce, balance, code_hash, new_gen);
            batches.accounts.push((*address, Some(encoded)));
        }

        // Storage writes with generation in key
        for (slot, value) in &update.storage {
            let key = StorageKey::new(*address, new_gen, *slot);
            batches.storage.push((key, if value.is_zero() { None } else { Some(*value) }));
        }
    }
}
```

---

## 5. Ledger Snapshot Management

**File:** `crates/node/ledger/src/lib.rs`

### Snapshot Type

```rust
pub type LedgerSnapshot = Snapshot<OverlayState<QmdbState>>;
```

Each snapshot contains:
- `parent: Option<ConsensusDigest>` — parent block digest
- `state: OverlayState<QmdbState>` — state at this block
- `state_root: StateRoot` — computed transition root
- `changes: ChangeSet` — changes from parent
- `tx_ids: BTreeSet<TxId>` — transactions in this block

### Snapshot Lifecycle

```
Genesis: OverlayState(QmdbState, empty changes)
    ↓
Block proposed → execute txs → OverlayState(parent_state, new_changes)
    ↓
Block verified → cached in snapshot store
    ↓
Block finalized → persist_snapshot()
    ├── QMDB commit (changes written to disk)
    └── Snapshot compacted (overlay cleared, fresh QMDB state)
```

### Snapshot Persistence (Lines 289-333)

```
LOCK 1: Get unpersisted chain, mark "persisting"
UNLOCK 1
    ↓
qmdb.commit_changes(changes)  ← NO LOCK (async, can take ms-seconds)
    ↓
LOCK 2: Clear "persisting", compact snapshots
UNLOCK 2
```

After persistence, the snapshot's overlay is replaced with a fresh one based on the updated QMDB state, freeing the accumulated changeset memory.

---

## 6. Potential Race Conditions

### A. Overlay Immutability

The `Arc<ChangeSet>` in OverlayState is immutable after creation. Multiple tasks can read from the same overlay safely. New changes create new OverlayState instances rather than mutating existing ones. This is safe.

### B. Code Lookup O(n)

The `code()` method iterates all accounts in the changeset to find matching code_hash. Under high throughput with many contract deployments, this could become a bottleneck.

### C. Generation Counter Saturation

`saturating_add(1)` means generation stops incrementing at `u64::MAX`. After that, new storage keys could collide with old ones. Practically impossible (requires 2^64 selfdestructs) but worth noting.

### D. State Root Divergence Vectors

1. **Different overlay bases**: If one validator has persisted while another hasn't, their overlay bases differ. The transition root computation uses the overlay's `compute_root()` which merges overlay changes with new changes, so the result should be the same regardless of persistence state. This is safe as long as the merge is deterministic.

2. **Nonce checking gap**: The overlay's nonce includes pending (unpersisted) changes, but `TransactionValidator` reads from QMDB (persisted only). A transaction could pass validation against QMDB nonce but fail during execution against the overlay nonce.

---

## 7. Key File References

| Component | File | Key Lines |
|-----------|------|-----------|
| OverlayState | `crates/storage/overlay/src/overlay.rs` | 8-153 |
| ChangeSet | `crates/storage/qmdb/src/changes.rs` | 9-99 |
| State root transition | `crates/storage/qmdb/src/root.rs` | 25-61 |
| QMDB store | `crates/storage/qmdb/src/store.rs` | 14-188 |
| Account encoding | `crates/storage/qmdb/src/encoding.rs` | 52-59 |
| Storage key | `crates/storage/qmdb/src/encoding.rs` | 16-39 |
| Ledger snapshots | `crates/node/ledger/src/lib.rs` | 29, 276-333 |
