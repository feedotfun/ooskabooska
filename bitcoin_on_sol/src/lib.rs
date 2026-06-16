use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;
pub mod tree;
pub mod util;

use instructions::*;

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

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

    // ---------------- Chest mint ----------------
    pub fn request_mint(ctx: Context<RequestMint>) -> Result<()> {
        instructions::mint::request_mint(ctx)
    }

    pub fn settle_mint(ctx: Context<SettleMint>, name: String, uri: String) -> Result<()> {
        instructions::mint::settle_mint(ctx, name, uri)
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

    pub fn set_team_name(ctx: Context<SetTeamName>, name: String) -> Result<()> {
        instructions::teams::set_team_name(ctx, name)
    }

    pub fn admin_kick_member(ctx: Context<AdminKickMember>) -> Result<()> {
        instructions::teams::admin_kick_member(ctx)
    }

    pub fn admin_disband_team(ctx: Context<AdminDisbandTeam>) -> Result<()> {
        instructions::teams::admin_disband_team(ctx)
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
