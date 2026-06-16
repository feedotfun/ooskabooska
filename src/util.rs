use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;

/// Deterministically expand a 32-byte VRF seed into the `n`-th pseudo-random
/// u64. Used to derive multiple independent draws (e.g. 3 small-block winners)
/// from a single Switchboard randomness reveal.
pub fn expand_u64(seed: &[u8; 32], n: u64) -> u64 {
    let mut buf = [0u8; 40];
    buf[..32].copy_from_slice(seed);
    buf[32..].copy_from_slice(&n.to_le_bytes());
    let h = keccak::hashv(&[&buf]);
    let mut out = [0u8; 8];
    out.copy_from_slice(&h.0[..8]);
    u64::from_le_bytes(out)
}

/// Read and validate a resolved Switchboard On-Demand randomness value.
///
/// Returns the revealed 32-byte value, or an error if the account does not
/// match `expected`, or the randomness has not yet been revealed.
pub fn read_randomness(
    randomness_ai: &AccountInfo,
    expected: &Pubkey,
) -> Result<[u8; 32]> {
    use crate::errors::BitcoinError;
    require_keys_eq!(
        *randomness_ai.key,
        *expected,
        BitcoinError::RandomnessAccountMismatch
    );

    let data = randomness_ai.try_borrow_data()?;
    let parsed = switchboard_on_demand::RandomnessAccountData::parse(data)
        .map_err(|_| error!(BitcoinError::RandomnessAccountMismatch))?;

    let clock = Clock::get()?;
    let value = parsed
        .get_value(&clock)
        .map_err(|_| error!(BitcoinError::RandomnessNotResolved))?;
    Ok(value)
}
