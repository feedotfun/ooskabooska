use anchor_lang::prelude::*;
use anchor_spl::token::Mint;

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::state::{MinerState, Team, TeamInvite};

#[derive(Accounts)]
#[instruction(id: Pubkey)]
pub struct CreateTeam<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + Team::INIT_SPACE,
        seeds = [SEED_TEAM, id.as_ref()],
        bump
    )]
    pub team: Account<'info, Team>,

    pub system_program: Program<'info, System>,
}

pub fn create_team(ctx: Context<CreateTeam>, id: Pubkey, name: String) -> Result<()> {
    require!(name.len() <= MAX_TEAM_NAME_LEN, BitcoinError::NameTooLong);
    let team = &mut ctx.accounts.team;
    team.authority = ctx.accounts.authority.key();
    team.id = id;
    team.name = name;
    team.total_active_hashrate = 0;
    team.acc_reward_per_hashrate = 0;
    team.member_count = 0;
    team.bump = ctx.bumps.team;
    Ok(())
}

/// Team authority invites a wallet. Invite-only membership.
#[derive(Accounts)]
#[instruction(invitee: Pubkey)]
pub struct InviteMember<'info> {
    #[account(mut, address = team.authority @ BitcoinError::Unauthorized)]
    pub authority: Signer<'info>,

    pub team: Account<'info, Team>,

    #[account(
        init,
        payer = authority,
        space = 8 + TeamInvite::INIT_SPACE,
        seeds = [SEED_INVITE, team.key().as_ref(), invitee.as_ref()],
        bump
    )]
    pub invite: Account<'info, TeamInvite>,

    pub system_program: Program<'info, System>,
}

pub fn invite_member(ctx: Context<InviteMember>, invitee: Pubkey) -> Result<()> {
    let invite = &mut ctx.accounts.invite;
    invite.team = ctx.accounts.team.key();
    invite.invitee = invitee;
    invite.bump = ctx.bumps.invite;
    Ok(())
}

/// Team authority revokes a previously issued invite.
#[derive(Accounts)]
#[instruction(invitee: Pubkey)]
pub struct RevokeInvite<'info> {
    #[account(mut, address = team.authority @ BitcoinError::Unauthorized)]
    pub authority: Signer<'info>,

    pub team: Account<'info, Team>,

    #[account(
        mut,
        close = authority,
        seeds = [SEED_INVITE, team.key().as_ref(), invitee.as_ref()],
        bump = invite.bump,
        constraint = invite.team == team.key() @ BitcoinError::InviteRequired
    )]
    pub invite: Account<'info, TeamInvite>,
}

pub fn revoke_invite(_ctx: Context<RevokeInvite>, _invitee: Pubkey) -> Result<()> {
    Ok(())
}

#[derive(Accounts)]
pub struct JoinTeam<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    pub nft_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [SEED_MINER, nft_mint.key().as_ref()],
        bump = miner_state.bump,
        constraint = miner_state.owner == owner.key() @ BitcoinError::NotNftOwner,
        constraint = !miner_state.has_team() @ BitcoinError::AlreadyInTeam
    )]
    pub miner_state: Account<'info, MinerState>,

    #[account(mut)]
    pub team: Account<'info, Team>,

    /// Proof that the team invited this wallet. Reusable across the owner's NFTs;
    /// the team authority can `revoke_invite` to remove access.
    #[account(
        seeds = [SEED_INVITE, team.key().as_ref(), owner.key().as_ref()],
        bump = invite.bump,
        constraint = invite.team == team.key() @ BitcoinError::InviteRequired,
        constraint = invite.invitee == owner.key() @ BitcoinError::InviteRequired
    )]
    pub invite: Account<'info, TeamInvite>,
}

pub fn join_team(ctx: Context<JoinTeam>) -> Result<()> {
    let miner = &mut ctx.accounts.miner_state;
    let team = &mut ctx.accounts.team;

    if miner.active {
        team.total_active_hashrate = team
            .total_active_hashrate
            .checked_add(miner.hashrate)
            .ok_or(BitcoinError::MathOverflow)?;
    }
    // Start the member with no claim on past team rewards.
    miner.team_reward_debt = team.acc_reward_per_hashrate;
    miner.team = team.key();
    team.member_count = team
        .member_count
        .checked_add(1)
        .ok_or(BitcoinError::MathOverflow)?;
    Ok(())
}

#[derive(Accounts)]
pub struct LeaveTeam<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    pub nft_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [SEED_MINER, nft_mint.key().as_ref()],
        bump = miner_state.bump,
        constraint = miner_state.owner == owner.key() @ BitcoinError::NotNftOwner
    )]
    pub miner_state: Account<'info, MinerState>,

    #[account(
        mut,
        constraint = miner_state.team == team.key() @ BitcoinError::NotInTeam
    )]
    pub team: Account<'info, Team>,
}

pub fn leave_team(ctx: Context<LeaveTeam>) -> Result<()> {
    let miner = &mut ctx.accounts.miner_state;
    let team = &mut ctx.accounts.team;

    // Realize earnings and remove active contribution.
    if miner.active {
        let owed = team
            .acc_reward_per_hashrate
            .checked_sub(miner.team_reward_debt)
            .ok_or(BitcoinError::MathOverflow)?
            .checked_mul(miner.hashrate as u128)
            .ok_or(BitcoinError::MathOverflow)?
            / ACC_SCALE;
        miner.pending = miner
            .pending
            .checked_add(owed as u64)
            .ok_or(BitcoinError::MathOverflow)?;
        team.total_active_hashrate = team
            .total_active_hashrate
            .checked_sub(miner.hashrate)
            .ok_or(BitcoinError::MathOverflow)?;
    }
    miner.team = Pubkey::default();
    miner.team_reward_debt = 0;
    team.member_count = team.member_count.saturating_sub(1);
    Ok(())
}

#[derive(Accounts)]
pub struct SetTeamName<'info> {
    #[account(address = team.authority @ BitcoinError::Unauthorized)]
    pub authority: Signer<'info>,
    #[account(mut)]
    pub team: Account<'info, Team>,
}

pub fn set_team_name(ctx: Context<SetTeamName>, name: String) -> Result<()> {
    require!(name.len() <= MAX_TEAM_NAME_LEN, BitcoinError::NameTooLong);
    ctx.accounts.team.name = name;
    Ok(())
}
