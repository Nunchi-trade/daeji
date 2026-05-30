//! State change tracking with merge capability.

use std::collections::{BTreeMap, HashMap};

use alloy_primitives::{Address, B256, U256};

/// Accumulated state changes that can be merged across blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// Account changes keyed by address.
    pub accounts: BTreeMap<Address, AccountUpdate>,
    /// Secondary index: code hash -> code bytes for O(1) code lookups.
    ///
    /// Populated automatically by [`insert`](Self::insert) and
    /// [`merge`](Self::merge) when an [`AccountUpdate`] carries deployed
    /// bytecode (`code: Some(...)`).
    pub code_by_hash: HashMap<B256, Vec<u8>>,
}

impl ChangeSet {
    /// Create an empty change set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are no changes.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    /// Number of accounts with changes.
    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    /// Merge another change set into this one.
    pub fn merge(&mut self, other: Self) {
        // Absorb the other set's code index entries.
        self.code_by_hash.extend(other.code_by_hash);

        for (address, update) in other.accounts {
            if let Some(existing) = self.accounts.get_mut(&address) {
                existing.merge(update);
            } else {
                self.accounts.insert(address, update);
            }
        }
    }

    /// Insert or update an account.
    pub fn insert(&mut self, address: Address, update: AccountUpdate) {
        // Populate the code index when new bytecode is present.
        if let Some(code) = &update.code {
            self.code_by_hash.insert(update.code_hash, code.clone());
        }

        if let Some(existing) = self.accounts.get_mut(&address) {
            existing.merge(update);
        } else {
            self.accounts.insert(address, update);
        }
    }
}

/// State changes for a single account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountUpdate {
    /// Whether account was created in this change.
    pub created: bool,
    /// Whether account was selfdestructed.
    pub selfdestructed: bool,
    /// Current nonce.
    pub nonce: u64,
    /// Current balance.
    pub balance: U256,
    /// Code hash.
    pub code_hash: B256,
    /// New code bytes (if code was deployed).
    pub code: Option<Vec<u8>>,
    /// Storage slot changes.
    pub storage: BTreeMap<U256, U256>,
}

impl AccountUpdate {
    /// Merge another update into this one.
    pub fn merge(&mut self, other: Self) {
        let Self { created, selfdestructed, nonce, balance, code_hash, code, storage } = other;

        if created {
            self.storage.clear();
            self.created = true;
        }

        if selfdestructed {
            self.storage.clear();
        }

        self.selfdestructed = selfdestructed;
        self.nonce = nonce;
        self.balance = balance;

        if self.code_hash != code_hash || code.is_some() {
            self.code = code;
        }
        self.code_hash = code_hash;

        if !selfdestructed {
            for (slot, value) in storage {
                self.storage.insert(slot, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_overwrites_nonce_and_balance() {
        let mut cs1 = ChangeSet::new();
        cs1.accounts.insert(
            Address::ZERO,
            AccountUpdate {
                created: false,
                selfdestructed: false,
                nonce: 1,
                balance: U256::from(100),
                code_hash: B256::ZERO,
                code: None,
                storage: BTreeMap::new(),
            },
        );

        let mut cs2 = ChangeSet::new();
        cs2.accounts.insert(
            Address::ZERO,
            AccountUpdate {
                created: false,
                selfdestructed: false,
                nonce: 5,
                balance: U256::from(500),
                code_hash: B256::ZERO,
                code: None,
                storage: BTreeMap::new(),
            },
        );

        cs1.merge(cs2);
        let update = cs1.accounts.get(&Address::ZERO).unwrap();
        assert_eq!(update.nonce, 5);
        assert_eq!(update.balance, U256::from(500));
    }

    #[test]
    fn selfdestruct_clears_storage() {
        let mut update = AccountUpdate {
            created: false,
            selfdestructed: false,
            nonce: 1,
            balance: U256::from(100),
            code_hash: B256::ZERO,
            code: None,
            storage: BTreeMap::from([(U256::from(1), U256::from(999))]),
        };

        update.merge(AccountUpdate {
            created: false,
            selfdestructed: true,
            nonce: 0,
            balance: U256::ZERO,
            code_hash: B256::ZERO,
            code: None,
            storage: BTreeMap::new(),
        });

        assert!(update.selfdestructed);
        assert!(update.storage.is_empty());
    }

    #[test]
    fn insert_populates_code_by_hash() {
        let mut cs = ChangeSet::new();
        let code_hash = B256::repeat_byte(0xAA);
        let code_bytes = vec![0x60, 0x00];
        cs.insert(
            Address::repeat_byte(0x01),
            AccountUpdate {
                created: true,
                selfdestructed: false,
                nonce: 0,
                balance: U256::ZERO,
                code_hash,
                code: Some(code_bytes.clone()),
                storage: BTreeMap::new(),
            },
        );
        assert_eq!(cs.code_by_hash.get(&code_hash).unwrap(), &code_bytes);
    }

    #[test]
    fn merge_propagates_code_by_hash() {
        let mut cs1 = ChangeSet::new();
        let mut cs2 = ChangeSet::new();

        let hash1 = B256::repeat_byte(0x11);
        let hash2 = B256::repeat_byte(0x22);
        let code1 = vec![0x01];
        let code2 = vec![0x02];

        cs1.insert(
            Address::repeat_byte(0x01),
            AccountUpdate {
                created: true,
                selfdestructed: false,
                nonce: 0,
                balance: U256::ZERO,
                code_hash: hash1,
                code: Some(code1.clone()),
                storage: BTreeMap::new(),
            },
        );
        cs2.insert(
            Address::repeat_byte(0x02),
            AccountUpdate {
                created: true,
                selfdestructed: false,
                nonce: 0,
                balance: U256::ZERO,
                code_hash: hash2,
                code: Some(code2.clone()),
                storage: BTreeMap::new(),
            },
        );

        cs1.merge(cs2);
        assert_eq!(cs1.code_by_hash.get(&hash1).unwrap(), &code1);
        assert_eq!(cs1.code_by_hash.get(&hash2).unwrap(), &code2);
    }
}
