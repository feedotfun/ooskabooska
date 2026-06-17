use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;
pub mod tree;
pub mod util;

use instructions::*;

declare_id!("8ozisczXr88dAct4ZKn82EKMMoRYc5kE7Z9ML7Z36gsm");

#[program]
pub mod bitcoin_on_sol {
    use super::*;

    // ---------------- Admin ----------------
    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        params: InitConfigParams,
    ) -> Result<()> {
        instructions::admin::initialize_config(ctx, params)
    }

    pub fn create_collection(
        ctx: Context<CreateCollection>,
        name: String,
        symbol: String,
        uri: String,
    ) -> Result<()> {
        instructions::admin::create_collection(ctx, name, symbol, uri)
    }

    pub fn fund_reward_pool(ctx: Context<FundRewardPool>, amount: u64) -> Result<()> {
        instructions::admin::fund_reward_pool(ctx, amount)
    }

    pub fn set_prices(
        ctx: Context<AdminOnly>,
        mint_price: u64,
        upgrade_cost: [u64; 4],
    ) -> Result<()> {
        instructions::admin::set_prices(ctx, mint_price, upgrade_cost)
    }

    pub fn set_emission(
        ctx: Context<AdminOnly>,
        base_small_reward: u64,
        base_big_reward: u64,
        halving_interval: u64,
    ) -> Result<()> {
        instructions::admin::set_emission(ctx, base_small_reward, base_big_reward, halving_interval)
    }

    pub fn set_crank_authority(ctx: Context<AdminOnly>, new_authority: Pubkey) -> Result<()> {
        instructions::admin::set_crank_authority(ctx, new_authority)
    }

    pub fn set_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
        instructions::admin::set_paused(ctx, paused)
    }

    pub fn set_team_params(
        ctx: Context<AdminOnly>,
        creation_fee_lamports: u64,
        max_members: u8,
        teams_enabled: bool,
    ) -> Result<()> {
        instructions::admin::set_team_params(ctx, creation_fee_lamports, max_members, teams_enabled)
    }

    pub fn set_block_timing(
        ctx: Context<AdminOnly>,
        small_interval: i64,
        big_interval: i64,
    ) -> Result<()> {
        instructions::admin::set_block_timing(ctx, small_interval, big_interval)
    }

    pub fn set_reward_bps(
        ctx: Context<AdminOnly>,
        small_min: u16,
        small_max: u16,
        big_min: u16,
        big_max: u16,
    ) -> Result<()> {
        instructions::admin::set_reward_bps(ctx, small_min, small_max, big_min, big_max)
    }

    pub fn set_emission_base(ctx: Context<AdminOnly>, emission_base: u64) -> Result<()> {
        instructions::admin::set_emission_base(ctx, emission_base)
    }

    pub fn set_halving(ctx: Context<AdminOnly>, halving_interval: u64) -> Result<()> {
        instructions::admin::set_halving(ctx, halving_interval)
    }

    pub fn set_multiplier(ctx: Context<AdminOnly>, enabled: bool, bps: u32) -> Result<()> {
        instructions::admin::set_multiplier(ctx, enabled, bps)
    }

    pub fn set_tier_hashrate(ctx: Context<AdminOnly>, hashrate: [u64; 5]) -> Result<()> {
        instructions::admin::set_tier_hashrate(ctx, hashrate)
    }

    pub fn set_max_active_hr(ctx: Context<AdminOnly>, max_active_hr: u64) -> Result<()> {
        instructions::admin::set_max_active_hr(ctx, max_active_hr)
    }

    pub fn set_game_enabled(ctx: Context<AdminOnly>, enabled: bool) -> Result<()> {
        instructions::admin::set_game_enabled(ctx, enabled)
    }

    pub fn blacklist_add(ctx: Context<BlacklistAdd>, target: Pubkey) -> Result<()> {
        instructions::admin::blacklist_add(ctx, target)
    }

    pub fn blacklist_remove(ctx: Context<BlacklistRemove>, target: Pubkey) -> Result<()> {
        instructions::admin::blacklist_remove(ctx, target)
    }

    // ---------------- Chest mint ----------------
    pub fn request_mint(ctx: Context<RequestMint>) -> Result<()> {
        instructions::mint::request_mint(ctx)
    }

    pub fn settle_mint(ctx: Context<SettleMint>, name: String, uri: String) -> Result<()> {
        instructions::mint::settle_mint(ctx, name, uri)
    }

    /// Instant temporary mint (no VRF/Metaplex yet). Capped at 5 per wallet.
    pub fn dev_mint(ctx: Context<DevMint>, mint_index: u64) -> Result<()> {
        instructions::mint::dev_mint(ctx, mint_index)
    }

    /// Real single-tx Metaplex NFT mint (on-chain randomness). Capped at 5/wallet.
    pub fn mint_nft(ctx: Context<MintNft>, name: String, uri: String) -> Result<()> {
        instructions::mint::mint_nft(ctx, name, uri)
    }

    // ---------------- Miner lifecycle ----------------
    pub fn activate_miner(ctx: Context<Activate>) -> Result<()> {
        instructions::lifecycle::activate_miner(ctx)
    }

    pub fn deactivate_miner(ctx: Context<Deactivate>) -> Result<()> {
        instructions::lifecycle::deactivate_miner(ctx)
    }

    // ---------------- Teams ----------------
    pub fn create_team(ctx: Context<CreateTeam>, name: String) -> Result<()> {
        instructions::teams::create_team(ctx, name)
    }

    pub fn invite_member(
        ctx: Context<InviteMember>,
        invite_id: u64,
        invitee: Pubkey,
    ) -> Result<()> {
        instructions::teams::invite_member(ctx, invite_id, invitee)
    }

    pub fn revoke_invite(ctx: Context<RevokeInvite>, invite_id: u64) -> Result<()> {
        instructions::teams::revoke_invite(ctx, invite_id)
    }

    pub fn join_team(ctx: Context<JoinTeam>, invite_id: u64) -> Result<()> {
        instructions::teams::join_team(ctx, invite_id)
    }

    pub fn leave_team(ctx: Context<LeaveTeam>) -> Result<()> {
        instructions::teams::leave_team(ctx)
    }

    pub fn admin_kick_member(ctx: Context<AdminKickMember>) -> Result<()> {
        instructions::teams::admin_kick_member(ctx)
    }

    pub fn disband_team(ctx: Context<DisbandTeam>) -> Result<()> {
        instructions::teams::disband_team(ctx)
    }

    // ---------------- Sacrifice / upgrade ----------------
    pub fn sacrifice(ctx: Context<Sacrifice>, name: String, uri: String) -> Result<()> {
        instructions::sacrifice::sacrifice(ctx, name, uri)
    }

    // ---------------- Crank / blocks ----------------
    pub fn commit_block(ctx: Context<CommitBlock>, kind: u8, index: u64) -> Result<()> {
        instructions::crank::commit_block(ctx, kind, index)
    }

    pub fn settle_block(ctx: Context<SettleBlock>, kind: u8, index: u64) -> Result<()> {
        instructions::crank::settle_block(ctx, kind, index)
    }

    // ---------------- Claims ----------------
    pub fn collect_win(
        ctx: Context<CollectWin>,
        kind: u8,
        index: u64,
        winner_index: u8,
    ) -> Result<()> {
        instructions::claims::collect_win(ctx, kind, index, winner_index)
    }

    pub fn claim_rewards(ctx: Context<ClaimRewards>) -> Result<()> {
        instructions::claims::claim_rewards(ctx)
    }

    pub fn claim_team_rewards(ctx: Context<ClaimTeamRewards>) -> Result<()> {
        instructions::claims::claim_team_rewards(ctx)
    }
}
