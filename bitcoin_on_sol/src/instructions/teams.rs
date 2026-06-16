use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::events::{InviteCreated, TeamCreated, TeamDisbanded, TeamMembershipChanged};
use crate::state::{Config, Team, TeamInvite, TeamNameRegistry, UserState};

/// Initialize a freshly-created (init_if_needed) UserState if it is blank.
fn ensure_user_state(user_state: &mut UserState, owner: Pubkey, bump: u8) {
    if user_state.owner == Pubkey::default() {
        user_state.owner = owner;
        user_state.bump = bump;
    }
}

// ---------------------------------------------------------------------------
// Create team (one per wallet, optional SOL fee). The creator auto-joins.
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(name: String)]
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

    /// Reserves the name globally: seeded by the name, so a duplicate name makes
    /// this `init` fail and the whole instruction reverts.
    #[account(
        init,
        payer = authority,
        space = 8 + TeamNameRegistry::INIT_SPACE,
        seeds = [SEED_TEAM_NAME, name.as_bytes()],
        bump
    )]
    pub name_registry: Account<'info, TeamNameRegistry>,

    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + UserState::INIT_SPACE,
        seeds = [SEED_USER, authority.key().as_ref()],
        bump
    )]
    pub user_state: Account<'info, UserState>,

    /// Receives the creation fee. Must be the configured admin wallet.
    #[account(mut, address = config.admin @ BitcoinError::InvalidParam)]
    pub fee_destination: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn create_team(ctx: Context<CreateTeam>, name: String) -> Result<()> {
    require!(ctx.accounts.config.teams_enabled, BitcoinError::TeamsDisabled);
    require!(is_valid_team_name(&name), BitcoinError::InvalidTeamName);

    let user_state = &mut ctx.accounts.user_state;
    ensure_user_state(user_state, ctx.accounts.authority.key(), ctx.bumps.user_state);
    require!(!user_state.has_team(), BitcoinError::AlreadyInTeam);

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
    team.member_count = 1; // creator is the first member
    team.bump = ctx.bumps.team;

    let registry = &mut ctx.accounts.name_registry;
    registry.team = team.key();
    registry.bump = ctx.bumps.name_registry;

    // Creator auto-joins their own team.
    user_state.team = team.key();

    emit!(TeamCreated {
        team: team.key(),
        authority: team.authority,
        fee_paid: fee,
    });
    emit!(TeamMembershipChanged {
        team: team.key(),
        member: user_state.owner,
        joined: true,
        by_admin: false,
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

    /// Seeded by the unique invite id, so the same id can never be issued twice.
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
// Join / leave (wallet-level membership)
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(invite_id: u64)]
pub struct JoinTeam<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + UserState::INIT_SPACE,
        seeds = [SEED_USER, owner.key().as_ref()],
        bump
    )]
    pub user_state: Account<'info, UserState>,

    #[account(mut)]
    pub team: Account<'info, Team>,

    /// Whitelist proof: the invite must target this team and this wallet.
    #[account(
        seeds = [SEED_INVITE, &invite_id.to_le_bytes()],
        bump = invite.bump,
        constraint = invite.team == team.key() @ BitcoinError::InviteRequired,
        constraint = invite.invitee == owner.key() @ BitcoinError::InviteRequired
    )]
    pub invite: Account<'info, TeamInvite>,

    pub system_program: Program<'info, System>,
}

pub fn join_team(ctx: Context<JoinTeam>, _invite_id: u64) -> Result<()> {
    let max_members = ctx.accounts.config.max_team_members as u32;
    let user_state = &mut ctx.accounts.user_state;
    ensure_user_state(user_state, ctx.accounts.owner.key(), ctx.bumps.user_state);
    require!(!user_state.has_team(), BitcoinError::AlreadyInTeam);

    let team = &mut ctx.accounts.team;
    require!(team.member_count < max_members, BitcoinError::TeamFull);

    user_state.team = team.key();
    team.member_count = team
        .member_count
        .checked_add(1)
        .ok_or(BitcoinError::MathOverflow)?;

    emit!(TeamMembershipChanged {
        team: team.key(),
        member: user_state.owner,
        joined: true,
        by_admin: false,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct LeaveTeam<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_USER, owner.key().as_ref()],
        bump = user_state.bump
    )]
    pub user_state: Account<'info, UserState>,

    #[account(
        mut,
        constraint = user_state.team == team.key() @ BitcoinError::NotInTeam
    )]
    pub team: Account<'info, Team>,
}

pub fn leave_team(ctx: Context<LeaveTeam>) -> Result<()> {
    // The owner cannot abandon their own team; they must disband it instead.
    require!(
        ctx.accounts.team.authority != ctx.accounts.owner.key(),
        BitcoinError::OwnerCannotLeave
    );

    let team = &mut ctx.accounts.team;
    let member_owner = ctx.accounts.user_state.owner;

    // Membership is wallet-level; already-active miners keep their snapshotted
    // team contribution and detach cleanly when deactivated.
    ctx.accounts.user_state.team = Pubkey::default();
    team.member_count = team.member_count.saturating_sub(1);

    emit!(TeamMembershipChanged {
        team: team.key(),
        member: member_owner,
        joined: false,
        by_admin: false,
    });
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
        seeds = [SEED_USER, member.key().as_ref()],
        bump = user_state.bump,
        constraint = user_state.team == team.key() @ BitcoinError::NotInTeam
    )]
    pub user_state: Account<'info, UserState>,

    /// CHECK: The wallet being kicked; only used to derive its UserState PDA.
    pub member: UncheckedAccount<'info>,

    #[account(mut)]
    pub team: Account<'info, Team>,
}

pub fn admin_kick_member(ctx: Context<AdminKickMember>) -> Result<()> {
    let team = &mut ctx.accounts.team;
    let member_owner = ctx.accounts.user_state.owner;
    ctx.accounts.user_state.team = Pubkey::default();
    team.member_count = team.member_count.saturating_sub(1);

    emit!(TeamMembershipChanged {
        team: team.key(),
        member: member_owner,
        joined: false,
        by_admin: true,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct DisbandTeam<'info> {
    /// Either the team owner or the program admin may disband.
    pub authority: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    /// Disband only when the owner is the last member (member_count == 1).
    #[account(
        mut,
        close = team_authority,
        constraint = team.member_count <= 1 @ BitcoinError::TeamNotEmpty,
        constraint = authority.key() == team.authority || authority.key() == config.admin
            @ BitcoinError::Unauthorized
    )]
    pub team: Account<'info, Team>,

    /// Frees the reserved name so it can be used again.
    #[account(
        mut,
        close = team_authority,
        seeds = [SEED_TEAM_NAME, team.name.as_bytes()],
        bump = name_registry.bump,
        constraint = name_registry.team == team.key() @ BitcoinError::InvalidParam
    )]
    pub name_registry: Account<'info, TeamNameRegistry>,

    /// The owner's UserState, cleared so the owner is no longer "in a team".
    #[account(
        mut,
        seeds = [SEED_USER, team.authority.as_ref()],
        bump = owner_state.bump
    )]
    pub owner_state: Account<'info, UserState>,

    /// CHECK: Receives the rent refund; must be the team owner.
    #[account(mut, address = team.authority @ BitcoinError::InvalidParam)]
    pub team_authority: UncheckedAccount<'info>,
}

pub fn disband_team(ctx: Context<DisbandTeam>) -> Result<()> {
    // Clear the owner's membership before the team account is closed.
    ctx.accounts.owner_state.team = Pubkey::default();

    emit!(TeamDisbanded {
        team: ctx.accounts.team.key(),
        authority: ctx.accounts.team.authority,
    });
    Ok(())
}
