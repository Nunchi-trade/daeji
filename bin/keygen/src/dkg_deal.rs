//! Trusted dealer DKG for devnet.
//!
//! Generates all BLS12-381 threshold shares using a single trusted dealer.
//! This is NOT secure for production but allows testing the validator workflow.

use std::{fs, io::Write as _, path::PathBuf};

use clap::Args;
use commonware_codec::{ReadExt, Write as _};
use commonware_cryptography::bls12381::{
    dkg::feldman_desmedt as dkg,
    primitives::{sharing::Mode, variant::MinSig},
};
use commonware_utils::{Faults, N3f1, TryCollect, ordered::Set};
use eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};

#[derive(Args, Debug)]
pub(crate) struct DkgDealArgs {
    #[arg(long, default_value = "4")]
    pub validators: usize,

    #[arg(long, default_value = "/shared")]
    pub output_dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct OutputJson {
    group_public_key: String,
    public_polynomial: String,
    threshold: u32,
    participants: usize,
    participant_keys: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct ShareJson {
    index: u32,
    secret: String,
}

pub(crate) fn run(args: DkgDealArgs) -> Result<()> {
    // Idempotency guard: if all validators already have DKG output, skip
    // generation entirely.  This prevents key rotation on every restart
    // (e.g. `docker compose stop` / `start`) which would cause a permanent
    // consensus deadlock because validators would hold mismatched BLS shares.
    let all_exist = (0..args.validators).all(|i| {
        let node_dir = args.output_dir.join(format!("node{}", i));
        node_dir.join("share.key").exists() && node_dir.join("output.json").exists()
    });

    if all_exist {
        tracing::info!(
            validators = args.validators,
            "DKG output already exists for all validators, skipping generation"
        );
        return Ok(());
    }

    let quorum = N3f1::quorum(args.validators);
    tracing::info!(
        validators = args.validators,
        quorum = quorum,
        max_faulty = args.validators as u32 - quorum,
        "Running trusted dealer DKG (quorum determined by N3f1: need {} of {} validators)",
        quorum,
        args.validators
    );

    let mut participants = Vec::with_capacity(args.validators);
    for i in 0..args.validators {
        let node_dir = args.output_dir.join(format!("node{}", i));
        let setup_path = node_dir.join("setup.json");

        let setup_str = fs::read_to_string(&setup_path)
            .wrap_err_with(|| format!("Failed to read setup.json for node{}", i))?;
        let setup: serde_json::Value = serde_json::from_str(&setup_str)?;

        let pk_hex = setup["public_key"]
            .as_str()
            .ok_or_else(|| eyre::eyre!("missing public_key in setup.json"))?;

        let pk_bytes = hex::decode(pk_hex)?;
        let pk = commonware_cryptography::ed25519::PublicKey::read(&mut pk_bytes.as_slice())
            .map_err(|e| eyre::eyre!("Failed to decode public key: {:?}", e))?;

        participants.push(pk);
        tracing::debug!(node = i, pk = %pk_hex, "Loaded participant");
    }

    let participants_set: Set<commonware_cryptography::ed25519::PublicKey> = participants
        .iter()
        .cloned()
        .try_collect()
        .map_err(|_| eyre::eyre!("Duplicate participants"))?;

    let participant_keys: Vec<String> = participants_set
        .iter()
        .map(|pk| {
            let mut bytes = Vec::new();
            pk.write(&mut bytes);
            hex::encode(bytes)
        })
        .collect();

    let mut rng = rand::rngs::OsRng;

    tracing::info!("Generating BLS threshold key shares");
    let (public_output, shares) =
        dkg::deal::<MinSig, _, N3f1>(&mut rng, Mode::default(), participants_set)
            .map_err(|e| eyre::eyre!("DKG deal failed: {:?}", e))?;

    let sharing = public_output.public();

    let mut public_polynomial_bytes = Vec::new();
    sharing.write(&mut public_polynomial_bytes);

    let group_key = sharing.public();
    let mut group_key_bytes = Vec::new();
    group_key.write(&mut group_key_bytes);

    tracing::info!(
        group_key = hex::encode(&group_key_bytes),
        polynomial_len = public_polynomial_bytes.len(),
        "Generated group public key and polynomial"
    );

    for (i, pk) in participants.iter().enumerate() {
        let share =
            shares.get_value(pk).ok_or_else(|| eyre::eyre!("Missing share for node{}", i))?;

        let mut share_bytes = Vec::new();
        share.write(&mut share_bytes);

        let node_dir = args.output_dir.join(format!("node{}", i));

        let output_json = OutputJson {
            group_public_key: hex::encode(&group_key_bytes),
            public_polynomial: hex::encode(&public_polynomial_bytes),
            threshold: quorum,
            participants: args.validators,
            participant_keys: participant_keys.clone(),
        };
        let output_path = node_dir.join("output.json");
        fs::write(&output_path, serde_json::to_string_pretty(&output_json)?)?;

        let share_json = ShareJson { index: share.index.get(), secret: hex::encode(&share_bytes) };
        let share_path = node_dir.join("share.key");
        write_secret_file(&share_path, serde_json::to_string_pretty(&share_json)?.as_bytes())?;

        tracing::info!(node = i, "Wrote DKG output and share");
    }

    tracing::info!("Trusted dealer DKG complete");
    tracing::info!("  Validators: {}", args.validators);
    tracing::info!("  Quorum (N3f1): {}", quorum);

    Ok(())
}

/// Write `data` to `path` with mode `0600` so key material is never world-readable.
fn write_secret_file(path: &std::path::Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .wrap_err_with(|| format!("Failed to create secret file {}", path.display()))?;
    f.write_all(data).wrap_err_with(|| format!("Failed to write secret file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::{self, SetupArgs};

    /// Regression test for issue #4 (DKG init-config race).
    ///
    /// Verifies that `keygen dkg-deal` is idempotent: a second invocation when
    /// all validators already have `share.key` and `output.json` must skip
    /// generation entirely, preserving the original files unchanged.
    ///
    /// Without this guard each `docker compose start` would call `dkg-deal`
    /// again, generating a fresh random polynomial.  Some validators would
    /// read the new shares while others still held the old shares, making it
    /// impossible to form a 3-of-4 threshold quorum and producing a permanent
    /// consensus deadlock.
    #[test]
    fn dkg_deal_is_idempotent_when_output_already_exists() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Bootstrap: generate identity keys with `keygen setup` so that
        // dkg-deal can find the setup.json files it needs.
        setup::run(SetupArgs {
            validators: 4,
            secondary_peers: 0,
            chain_id: 1337,
            output_dir: dir.path().to_path_buf(),
            base_port: 30303,
        })
        .expect("setup run");

        let args = || DkgDealArgs { validators: 4, output_dir: dir.path().to_path_buf() };

        // First run: generate DKG shares.
        run(args()).expect("first dkg-deal run");

        // Capture the share.key and output.json contents for all nodes.
        let snapshots: Vec<(Vec<u8>, Vec<u8>)> = (0..4)
            .map(|i| {
                let node_dir = dir.path().join(format!("node{}", i));
                let share = fs::read(node_dir.join("share.key")).expect("read share.key");
                let output = fs::read(node_dir.join("output.json")).expect("read output.json");
                (share, output)
            })
            .collect();

        // Second run: must skip entirely because all outputs already exist.
        run(args()).expect("second dkg-deal run");

        // Verify every node still holds the original key material.
        for (i, (orig_share, orig_output)) in snapshots.iter().enumerate() {
            let node_dir = dir.path().join(format!("node{}", i));
            let share =
                fs::read(node_dir.join("share.key")).expect("read share.key after second run");
            let output =
                fs::read(node_dir.join("output.json")).expect("read output.json after second run");
            assert_eq!(
                share, *orig_share,
                "node{i} share.key must not change across dkg-deal runs"
            );
            assert_eq!(
                output, *orig_output,
                "node{i} output.json must not change across dkg-deal runs"
            );
        }
    }
}
