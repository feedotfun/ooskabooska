use anchor_lang::prelude::*;

use crate::constants::*;

/// Global program configuration and live counters. Singleton PDA [SEED_CONFIG].
#[account]
#[derive(InitSpace)]
pub struct Config {
    pub admin: Pubkey,
    pub crank_authority: Pubkey,
    /// The (pump.fun) SPL token mint used for burns and rewards.
    pub token_mint: Pubkey,
    /// PDA-owned ATA holding the 400M reward pool.
    pub reward_vault: Pubkey,
    /// Verified Metaplex collection the 2,100 NFTs belong to.
    pub collection_mint: Pubkey,
    /// Switchboard On-Demand queue used for randomness.
    pub switchboard_queue: Pubkey,
    /// The (keypair-owned) zero-copy lottery tree account.
    pub miner_tree: Pubkey,

    /// Tokens burned per chest mint (base units).
    pub mint_price: u64,
    /// Token cost burned per sacrifice, indexed by source tier (0..=2 used).
    pub upgrade_cost: [u64; 4],

    /// Base small/big block rewards (base units) before halving.
    pub base_small_reward: u64,
    pub base_big_reward: u64,
    /// Cycles per halving (a "cycle" = one big block). 0 disables halving.
    pub halving_interval: u64,

    // Live counters
    pub total_burned: u64,
    pub pool_remaining: u64,
    pub cycle_index: u64,
    pub small_block_index: u64,
    pub big_block_index: u64,
    pub last_small_ts: i64,
    pub last_big_ts: i64,
    pub tier_remaining: [u32; TIER_COUNT],
    pub minted_total: u32,

    // Team settings (admin-configurable)
    /// Lamports charged on team creation (0 = free).
    pub team_creation_fee_lamports: u64,
    /// Maximum members allowed per team.
    pub max_team_members: u8,
    /// Whether new teams may be created.
    pub teams_enabled: bool,

    // Block timing + emission (admin-configurable)
    /// Seconds between small / big blocks.
    pub small_interval: i64,
    pub big_interval: i64,
    /// Per-block reward range in basis points of `emission_base` (random via VRF).
    pub small_bps_min: u16,
    pub small_bps_max: u16,
    pub big_bps_min: u16,
    pub big_bps_max: u16,
    /// Base amount (token base units) that block reward percentages apply to.
    pub emission_base: u64,
    /// Total settled blocks (small + big); drives halving.
    pub total_blocks: u64,
    /// Optional global reward multiplier (basis points; 10000 = 1x).
    pub global_multiplier_bps: u32,
    pub multiplier_enabled: bool,
    /// Hashrate per tier (admin nerf/buff). Index by tier id.
    pub tier_hashrate: [u64; TIER_COUNT],
    /// Max active hashrate a single wallet may run at once.
    pub max_active_hr: u64,
    /// Master on/off switch for the mining game.
    pub game_enabled: bool,

    pub paused: bool,
    pub bump: u8,
    pub vault_auth_bump: u8,
    pub mint_auth_bump: u8,
}

impl Config {
    /// Reward for the next block of the given kind, applying halving and
    /// clamping to the remaining pool.
    pub fn block_reward(&self, kind: u8) -> u64 {
        let base = if kind == BLOCK_KIND_BIG {
            self.base_big_reward
        } else {
            self.base_small_reward
        };
        let halvings = if self.halving_interval == 0 {
            0
        } else {
            (self.cycle_index / self.halving_interval).min(63)
        };
        let scaled = base >> halvings;
        scaled.min(self.pool_remaining)
    }
}

/// Per-user state. PDA [SEED_USER, owner].
#[account]
#[derive(InitSpace)]
pub struct UserState {
    pub owner: Pubkey,
    pub active_count: u16,
    pub mint_nonce: u64,
    pub total_minted: u32,
    /// The team this wallet belongs to, or Pubkey::default() when not in a team.
    /// Membership is per wallet; a miner inherits this team when it is activated.
    pub team: Pubkey,
    pub bump: u8,
    /// Sum of hashrate of this wallet's currently-active miners (capped).
    pub active_hashrate: u64,
}

impl UserState {
    pub fn has_team(&self) -> bool {
        self.team != Pubkey::default()
    }
}

/// Per-NFT mining state. PDA [SEED_MINER, nft_mint].
#[account]
#[derive(InitSpace)]
pub struct MinerState {
    pub owner: Pubkey,
    pub nft_mint: Pubkey,
    pub tier: u8,
    pub hashrate: u64,
    pub active: bool,
    /// Slot in the lottery tree while active; 0 when inactive.
    pub tree_slot: u32,
    /// Team PDA, or Pubkey::default() when solo.
    pub team: Pubkey,
    /// Checkpoint of the team accumulator at last interaction.
    pub team_reward_debt: u128,
    /// Unclaimed solo rewards (base units).
    pub pending: u64,
    pub blocks_won: u64,
    pub total_earned: u64,
    pub created_at: i64,
    /// Unix time before which an active miner may not be deactivated.
    pub lock_until: i64,
    pub bump: u8,
}

impl MinerState {
    pub fn has_team(&self) -> bool {
        self.team != Pubkey::default()
    }
}

/// A mining pool. One team per wallet: PDA [SEED_TEAM, authority].
#[account]
#[derive(InitSpace)]
pub struct Team {
    pub authority: Pubkey,
    #[max_len(MAX_TEAM_NAME_LEN)]
    pub name: String,
    pub total_active_hashrate: u64,
    /// Accumulated reward per unit of hashrate, scaled by ACC_SCALE.
    pub acc_reward_per_hashrate: u128,
    pub member_count: u32,
    pub bump: u8,
}

/// Marks a wallet as blacklisted. Existence of PDA [SEED_BLACKLIST, wallet]
/// means the wallet is banned from minting / activating / creating teams.
#[account]
#[derive(InitSpace)]
pub struct Blacklist {
    pub wallet: Pubkey,
    pub bump: u8,
}

/// Registry entry that reserves a team name globally. PDA
/// [SEED_TEAM_NAME, name] - because the name seeds the PDA, two teams can never
/// share a name (the second `create_team` fails at init).
#[account]
#[derive(InitSpace)]
pub struct TeamNameRegistry {
    pub team: Pubkey,
    pub bump: u8,
}

/// An invitation (whitelist entry) allowing `invitee` to join `team`.
/// Invite-only membership. PDA [SEED_INVITE, invite_id] - the unique 10-digit
/// id seeds the PDA, so two invites can never share an id.
#[account]
#[derive(InitSpace)]
pub struct TeamInvite {
    pub team: Pubkey,
    pub invitee: Pubkey,
    pub invite_id: u64,
    pub bump: u8,
}

/// A pending chest mint awaiting randomness. PDA [SEED_PENDING_MINT, user, nonce].
#[account]
#[derive(InitSpace)]
pub struct PendingMint {
    pub user: Pubkey,
    pub nonce: u64,
    pub randomness: Pubkey,
    pub commit_slot: u64,
    pub settled: bool,
    pub bump: u8,
}

/// A single recorded block winner.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, InitSpace)]
pub struct Winner {
    pub nft_mint: Pubkey,
    pub collected: bool,
}

/// A block round (small or big). PDA [SEED_ROUND, kind, index].
#[account]
#[derive(InitSpace)]
pub struct BlockRound {
    pub kind: u8,
    pub index: u64,
    pub randomness: Pubkey,
    pub commit_slot: u64,
    pub reward_each: u64,
    pub winner_count: u8,
    #[max_len(MAX_WINNERS)]
    pub winners: Vec<Winner>,
    pub settled: bool,
    pub bump: u8,
}
