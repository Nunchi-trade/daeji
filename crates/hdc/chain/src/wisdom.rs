//! WisdomGate -- submit/challenge/resolve lifecycle for knowledge validation.
//!
//! Manages the lifecycle of knowledge submissions: submit -> challenge window
//! -> accept/reject. Uses `BTreeMap` for deterministic iteration in `resolve()`.

use std::collections::BTreeMap;

use alloy_primitives::{Address, B256};

/// Errors from WisdomGate operations.
#[derive(Debug, thiserror::Error)]
pub enum WisdomError {
    /// A submission with this ID already exists.
    #[error("duplicate submission: {0}")]
    DuplicateSubmission(B256),
    /// The referenced submission was not found.
    #[error("submission not found: {0}")]
    NotFound(B256),
    /// The submission is not in a valid state for the requested operation.
    #[error("invalid state transition: expected {expected:?}, got {actual:?}")]
    InvalidState {
        /// The state required for this operation.
        expected: WisdomState,
        /// The actual state of the submission.
        actual: WisdomState,
    },
}

/// A submission to the WisdomGate for validation.
#[derive(Debug, Clone)]
pub struct WisdomSubmission {
    /// The vector being submitted.
    pub vector_id: B256,
    /// The submitter.
    pub submitter: Address,
    /// Block number of submission.
    pub submitted_at: u64,
    /// Current lifecycle state.
    pub state: WisdomState,
    /// Challenge bond deposited.
    pub bond: u64,
    /// Number of challenges received.
    pub challenge_count: u32,
}

/// WisdomGate lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WisdomState {
    /// Pending validation (waiting for challenge window).
    Pending,
    /// Under active challenge.
    Challenged,
    /// Accepted into the knowledge commons.
    Accepted,
    /// Rejected by the community.
    Rejected,
}

/// WisdomGate manager.
///
/// Uses `BTreeMap` so that `resolve()` iterates in deterministic key order.
#[derive(Debug, Default)]
pub struct WisdomGate {
    submissions: BTreeMap<B256, WisdomSubmission>,
    /// Challenge window in blocks.
    challenge_window: u64,
}

impl WisdomGate {
    /// Create a new WisdomGate with the given challenge window (in blocks).
    pub fn new(challenge_window: u64) -> Self {
        Self { submissions: BTreeMap::new(), challenge_window }
    }

    /// Submit a vector for validation.
    ///
    /// Returns the vector ID on success, or an error if a submission with
    /// this ID already exists.
    pub fn submit(
        &mut self,
        vector_id: B256,
        submitter: Address,
        block_number: u64,
        bond: u64,
    ) -> Result<B256, WisdomError> {
        if self.submissions.contains_key(&vector_id) {
            return Err(WisdomError::DuplicateSubmission(vector_id));
        }
        self.submissions.insert(
            vector_id,
            WisdomSubmission {
                vector_id,
                submitter,
                submitted_at: block_number,
                state: WisdomState::Pending,
                bond,
                challenge_count: 0,
            },
        );
        Ok(vector_id)
    }

    /// Challenge a pending submission.
    ///
    /// Returns `Ok(())` on success, or an error if the submission is not found
    /// or not in the `Pending` state.
    pub fn challenge(&mut self, vector_id: &B256, _challenger: Address) -> Result<(), WisdomError> {
        let sub = self.submissions.get_mut(vector_id).ok_or(WisdomError::NotFound(*vector_id))?;

        if sub.state != WisdomState::Pending {
            return Err(WisdomError::InvalidState {
                expected: WisdomState::Pending,
                actual: sub.state,
            });
        }
        sub.state = WisdomState::Challenged;
        sub.challenge_count += 1;
        Ok(())
    }

    /// Resolve submissions that have passed the challenge window.
    ///
    /// Iterates in deterministic (key-sorted) order.
    pub fn resolve(&mut self, current_block: u64) {
        for sub in self.submissions.values_mut() {
            if sub.state == WisdomState::Pending
                && current_block >= sub.submitted_at + self.challenge_window
            {
                sub.state = WisdomState::Accepted;
            }
        }
    }

    /// Get the state of a submission.
    pub fn get(&self, vector_id: &B256) -> Option<&WisdomSubmission> {
        self.submissions.get(vector_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submit_and_resolve() {
        let mut gate = WisdomGate::new(100);
        let id = B256::ZERO;
        let submitter = Address::ZERO;

        let result = gate.submit(id, submitter, 10, 1000);
        assert!(result.is_ok());
        assert_eq!(result.expect("just checked"), id);

        // Duplicate should fail.
        let dup = gate.submit(id, submitter, 10, 1000);
        assert!(matches!(dup, Err(WisdomError::DuplicateSubmission(_))));

        // Before challenge window.
        gate.resolve(50);
        assert_eq!(gate.get(&id).expect("should exist").state, WisdomState::Pending);

        // After challenge window.
        gate.resolve(111);
        assert_eq!(gate.get(&id).expect("should exist").state, WisdomState::Accepted);
    }

    #[test]
    fn test_challenge() {
        let mut gate = WisdomGate::new(100);
        let id = B256::ZERO;
        let submitter = Address::ZERO;
        let challenger = Address::new([0x01; 20]);

        gate.submit(id, submitter, 10, 1000).expect("submit should succeed");

        assert!(gate.challenge(&id, challenger).is_ok());
        assert_eq!(gate.get(&id).expect("should exist").state, WisdomState::Challenged);

        // Challenge window passes but state is Challenged, not Pending.
        gate.resolve(200);
        assert_eq!(gate.get(&id).expect("should exist").state, WisdomState::Challenged);
    }

    #[test]
    fn test_challenge_not_found() {
        let mut gate = WisdomGate::new(100);
        let bogus = B256::from([0xFF; 32]);
        let challenger = Address::ZERO;
        let result = gate.challenge(&bogus, challenger);
        assert!(matches!(result, Err(WisdomError::NotFound(_))));
    }

    #[test]
    fn test_challenge_wrong_state() {
        let mut gate = WisdomGate::new(100);
        let id = B256::ZERO;
        let submitter = Address::ZERO;
        let challenger = Address::new([0x01; 20]);

        gate.submit(id, submitter, 10, 1000).expect("submit should succeed");
        // Resolve to Accepted.
        gate.resolve(111);

        let result = gate.challenge(&id, challenger);
        assert!(matches!(
            result,
            Err(WisdomError::InvalidState {
                expected: WisdomState::Pending,
                actual: WisdomState::Accepted,
            })
        ));
    }
}
