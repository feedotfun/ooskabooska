use anchor_lang::prelude::*;

/// Number of rarity tiers (Common, Uncommon, Rare, Legendary, Grail).
pub const TIER_COUNT: usize = 5;

/// Tier indices.
pub const TIER_COMMON: u8 = 0;
pub const TIER_UNCOMMON: u8 = 1;
pub const TIER_RARE: u8 = 2;
pub const TIER_LEGENDARY: u8 = 3;
pub const TIER_GRAIL: u8 = 4;

/// Hashrate (HR/s) per tier. Index by tier id.
pub const TIER_HASHRATE: [u64; TIER_COUNT] = [10, 20, 40, 80, 400];

/// Initial mintable supply per tier. Total = 2,100.
/// Common 1260, Uncommon 630, Rare 147, Legendary 57, Grail 6.
pub const INITIAL_TIER_REMAINING: [u32; TIER_COUNT] = [1260, 630, 147, 57, 6];

/// Total NFT supply that can be minted from chests.
pub const TOTAL_NFT_SUPPLY: u32 = 2100;

/// Highest tier reachable through sacrifice (Grail is mint-only).
pub const MAX_SACRIFICE_RESULT_TIER: u8 = TIER_LEGENDARY;

/// Mining cadence, in seconds.
pub const SMALL_BLOCK_INTERVAL: i64 = 5 * 60; // 5 minutes
pub const BIG_BLOCK_INTERVAL: i64 = 30 * 60; // 30 minutes

/// Winners drawn per block type.
pub const SMALL_BLOCK_WINNERS: u8 = 3;
pub const BIG_BLOCK_WINNERS: u8 = 1;

/// Block kinds (stored in BlockRound).
pub const BLOCK_KIND_SMALL: u8 = 0;
pub const BLOCK_KIND_BIG: u8 = 1;

/// Max NFTs a single user may have active (mining) at once.
pub const MAX_ACTIVE_PER_USER: u16 = 5;

/// Once a miner is activated it is locked (cannot be deactivated, and therefore
/// cannot be sacrificed) for this many seconds.
pub const ACTIVATION_LOCK_SECONDS: i64 = 12 * 60 * 60; // 12 hours

/// Capacity of the active-miner lottery tree. Comfortably above 2,100.
pub const MINER_TREE_CAP: usize = 2560;

/// Fixed-point scale for the team reward-per-hashrate accumulator.
pub const ACC_SCALE: u128 = 1_000_000_000_000; // 1e12

/// Max length (bytes) for a team name.
pub const MAX_TEAM_NAME_LEN: usize = 32;

/// Default lamports charged to create a team (0.01 SOL). Admin-configurable;
/// set to 0 to make team creation free.
pub const DEFAULT_TEAM_CREATION_FEE_LAMPORTS: u64 = 10_000_000;

/// Default maximum members per team. Admin-configurable.
pub const DEFAULT_MAX_TEAM_MEMBERS: u8 = 5;

/// Invite ids are exactly 10-digit numbers, so they fall in [1e9, 1e10).
pub const INVITE_ID_MIN: u64 = 1_000_000_000;
pub const INVITE_ID_MAX: u64 = 9_999_999_999;

/// Validates a team name: 1..=32 bytes, ASCII letters and digits only.
pub fn is_valid_team_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_TEAM_NAME_LEN
        && name.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Validates that an invite id is a 10-digit number.
pub fn is_valid_invite_id(id: u64) -> bool {
    (INVITE_ID_MIN..=INVITE_ID_MAX).contains(&id)
}

/// Max winners stored on a single BlockRound (covers small block of 3).
pub const MAX_WINNERS: usize = 3;

// ---- PDA seeds ----
pub const SEED_CONFIG: &[u8] = b"config";
pub const SEED_USER: &[u8] = b"user";
pub const SEED_MINER: &[u8] = b"miner";
pub const SEED_TEAM: &[u8] = b"team";
pub const SEED_TREE: &[u8] = b"tree";
pub const SEED_PENDING_MINT: &[u8] = b"pmint";
pub const SEED_ROUND: &[u8] = b"round";
pub const SEED_VAULT_AUTH: &[u8] = b"vault_auth";
pub const SEED_MINT_AUTH: &[u8] = b"mint_auth";
pub const SEED_INVITE: &[u8] = b"invite";

/// Returns the tier id for a uniformly random value given the remaining
/// per-tier supply. The pick is weighted by remaining counts, which makes the
/// launch odds equal to the tier caps (60/30/7/2.71/0.29) and guarantees the
/// final distribution exactly fills [1260, 630, 147, 57, 6].
///
/// `rand` is any u64; `remaining` is the live per-tier remaining counts.
/// Returns `None` only if every tier is exhausted (supply fully minted).
pub fn weighted_tier(rand: u64, remaining: &[u32; TIER_COUNT]) -> Option<u8> {
    let total: u64 = remaining.iter().map(|&c| c as u64).sum();
    if total == 0 {
        return None;
    }
    let mut pick = rand % total;
    for (i, &count) in remaining.iter().enumerate() {
        let c = count as u64;
        if pick < c {
            return Some(i as u8);
        }
        pick -= c;
    }
    // Unreachable because pick < total, but fall back to the last non-empty tier.
    for i in (0..TIER_COUNT).rev() {
        if remaining[i] > 0 {
            return Some(i as u8);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_tier_exactly_fills_supply() {
        let mut remaining = INITIAL_TIER_REMAINING;
        let mut minted = [0u32; TIER_COUNT];
        // Draw the full supply with a simple LCG; every pick must succeed and
        // the final distribution must equal the caps exactly.
        let mut seed: u64 = 0x1234_5678_9abc_def0;
        for _ in 0..TOTAL_NFT_SUPPLY {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let tier = weighted_tier(seed, &remaining).expect("supply not exhausted");
            let ti = tier as usize;
            remaining[ti] -= 1;
            minted[ti] += 1;
        }
        assert_eq!(minted, INITIAL_TIER_REMAINING);
        assert_eq!(remaining, [0; TIER_COUNT]);
        assert!(weighted_tier(0, &remaining).is_none());
    }
}
