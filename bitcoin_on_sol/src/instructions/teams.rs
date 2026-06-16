use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
use anchor_spl::token::Mint;

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::events::{
    InviteCreated, TeamCreated, TeamDisbanded, TeamMembershipChanged,
};
use crate::state::{Config, MinerState, Team, TeamInvite};

/// Realize a member's outstanding team earnings into its solo `pending` balance
/// and remove its active hashrate contribution from the team. Shared by
/// `leave_team` and `admin_kick_member`.
fn detach_miner_from_team(miner: &mut MinerState, team: &mut Team) -> Result<()> {
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

// ---------------------------------------------------------------------------
// Create team (one per wallet, optional SOL fee)
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct CreateTeam<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    /// One team per wallet: the authority seeds the PDA, so a second
    /// `create_team` from the same wallet fails at init.
    #[account(
        init,
        payer = authority,
        space = 8 + Team::INIT_SPACE,
        seeds = [SEED_TEAM, authority.key().as_ref()],
        bump
    )]
    pub team: Account<'info, Team>,

    /// Receives the creation fee. Must be the configured admin wallet.
    #[account(mut, address = config.admin @ BitcoinError::InvalidParam)]
    pub fee_destination: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn create_team(ctx: Context<CreateTeam>, name: String) -> Result<()> {
    require!(ctx.accounts.config.teams_enabled, BitcoinError::TeamsDisabled);
    require!(is_valid_team_name(&name), BitcoinError::InvalidTeamName);

    let fee = ctx.accounts.config.team_creation_fee_lamports;
    if fee > 0 {
        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: ctx.accounts.fee_destination.to_account_info(),
                },
            ),
            fee,
        )?;
    }

    let team = &mut ctx.accounts.team;
    team.authority = ctx.accounts.authority.key();
    team.name = name;
    team.total_active_hashrate = 0;
    team.acc_reward_per_hashrate = 0;
    team.member_count = 0;
    team.bump = ctx.bumps.team;

    emit!(TeamCreated {
        team: team.key(),
        authority: team.authority,
        fee_paid: fee,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Invites (whitelist a wallet with a unique 10-digit id)
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(invite_id: u64, invitee: Pubkey)]
pub struct InviteMember<'info> {
    #[account(mut, address = team.authority @ BitcoinError::Unauthorized)]
    pub authority: Signer<'info>,

    pub team: Account<'info, Team>,

    /// Seeded by the unique invite id, so the same id can never be issued twice
    /// (a second init at this address fails).
    #[account(
        init,
        payer = authority,
        space = 8 + TeamInvite::INIT_SPACE,
        seeds = [SEED_INVITE, &invite_id.to_le_bytes()],
        bump
    )]
    pub invite: Account<'info, TeamInvite>,

    pub system_program: Program<'info, System>,
}

pub fn invite_member(ctx: Context<InviteMember>, invite_id: u64, invitee: Pubkey) -> Result<()> {
    require!(is_valid_invite_id(invite_id), BitcoinError::InvalidInviteId);
    let invite = &mut ctx.accounts.invite;
    invite.team = ctx.accounts.team.key();
    invite.invitee = invitee;
    invite.invite_id = invite_id;
    invite.bump = ctx.bumps.invite;

    emit!(InviteCreated {
        team: invite.team,
        invitee,
        invite_id,
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(invite_id: u64)]
pub struct RevokeInvite<'info> {
    #[account(mut, address = team.authority @ BitcoinError::Unauthorized)]
    pub authority: Signer<'info>,

    pub team: Account<'info, Team>,

    #[account(
        mut,
        close = authority,
        seeds = [SEED_INVITE, &invite_id.to_le_bytes()],
        bump = invite.bump,
        constraint = invite.team == team.key() @ BitcoinError::InviteRequired
    )]
    pub invite: Account<'info, TeamInvite>,
}

pub fn revoke_invite(_ctx: Context<RevokeInvite>, _invite_id: u64) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Join / leave
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(invite_id: u64)]
pub struct JoinTeam<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

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

    /// Whitelist proof: the invite must target this team and this wallet.
    /// Reusable across the owner's NFTs; the team authority can `revoke_invite`.
    #[account(
        seeds = [SEED_INVITE, &invite_id.to_le_bytes()],
        bump = invite.bump,
        constraint = invite.team == team.key() @ BitcoinError::InviteRequired,
        constraint = invite.invitee == owner.key() @ BitcoinError::InviteRequired
    )]
    pub invite: Account<'info, TeamInvite>,
}

pub fn join_team(ctx: Context<JoinTeam>, _invite_id: u64) -> Result<()> {
    let max_members = ctx.accounts.config.max_team_members as u32;
    let miner = &mut ctx.accounts.miner_state;
    let team = &mut ctx.accounts.team;

    require!(team.member_count < max_members, BitcoinError::TeamFull);

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

    emit!(TeamMembershipChanged {
        team: team.key(),
        nft_mint: miner.nft_mint,
        owner: miner.owner,
        joined: true,
        by_admin: false,
    });
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
    let team_key = team.key();
    let nft_mint = miner.nft_mint;
    let owner = miner.owner;

    detach_miner_from_team(miner, team)?;

    emit!(TeamMembershipChanged {
        team: team_key,
        nft_mint,
        owner,
        joined: false,
        by_admin: false,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Team name
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct SetTeamName<'info> {
    #[account(address = team.authority @ BitcoinError::Unauthorized)]
    pub authority: Signer<'info>,
    #[account(mut)]
    pub team: Account<'info, Team>,
}

pub fn set_team_name(ctx: Context<SetTeamName>, name: String) -> Result<()> {
    require!(is_valid_team_name(&name), BitcoinError::InvalidTeamName);
    ctx.accounts.team.name = name;
    Ok(())
}

// ---------------------------------------------------------------------------
// Admin moderation: kick a member, disband a team
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct AdminKickMember<'info> {
    #[account(address = config.admin @ BitcoinError::Unauthorized)]
    pub admin: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        constraint = miner_state.team == team.key() @ BitcoinError::NotInTeam
    )]
    pub miner_state: Account<'info, MinerState>,

    #[account(mut)]
    pub team: Account<'info, Team>,
}

pub fn admin_kick_member(ctx: Context<AdminKickMember>) -> Result<()> {
    let miner = &mut ctx.accounts.miner_state;
    let team = &mut ctx.accounts.team;
    let team_key = team.key();
    let nft_mint = miner.nft_mint;
    let owner = miner.owner;

    detach_miner_from_team(miner, team)?;

    emit!(TeamMembershipChanged {
        team: team_key,
        nft_mint,
        owner,
        joined: false,
        by_admin: true,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct AdminDisbandTeam<'info> {
    #[account(address = config.admin @ BitcoinError::Unauthorized)]
    pub admin: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    /// Rent is refunded to the original team owner. Disband only when empty
    /// (kick all members first) so no miner is left pointing at a dead account.
    #[account(
        mut,
        close = team_authority,
        constraint = team.member_count == 0 @ BitcoinError::TeamNotEmpty
    )]
    pub team: Account<'info, Team>,

    /// CHECK: Receives the team's rent refund; must match the team owner.
    #[account(mut, address = team.authority @ BitcoinError::InvalidParam)]
    pub team_authority: UncheckedAccount<'info>,
}

pub fn admin_disband_team(ctx: Context<AdminDisbandTeam>) -> Result<()> {
    emit!(TeamDisbanded {
        team: ctx.accounts.team.key(),
        authority: ctx.accounts.team.authority,
    });
    Ok(())
}
