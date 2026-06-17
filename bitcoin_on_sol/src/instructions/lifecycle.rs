use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, TokenAccount};

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::events::{MinerActivated, MinerDeactivated};
use crate::state::{Config, MinerState, Team, UserState};
use crate::tree::MinerTree;
use crate::util::require_not_blacklisted;

#[derive(Accounts)]
pub struct Activate<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [SEED_CONFIG],
        bump = config.bump,
        constraint = !config.paused @ BitcoinError::Paused,
        constraint = config.game_enabled @ BitcoinError::GameDisabled
    )]
    pub config: Account<'info, Config>,

    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + UserState::INIT_SPACE,
        seeds = [SEED_USER, owner.key().as_ref()],
        bump
    )]
    pub user_state: Account<'info, UserState>,

    #[account(
        mut,
        seeds = [SEED_MINER, nft_mint.key().as_ref()],
        bump = miner_state.bump
    )]
    pub miner_state: Account<'info, MinerState>,

    pub nft_mint: Account<'info, Mint>,

    /// Proves the signer currently owns the NFT.
    #[account(
        constraint = nft_token.mint == nft_mint.key() @ BitcoinError::MintMismatch,
        constraint = nft_token.owner == owner.key() @ BitcoinError::NotNftOwner,
        constraint = nft_token.amount == 1 @ BitcoinError::NotNftOwner
    )]
    pub nft_token: Account<'info, TokenAccount>,

    #[account(mut, address = config.miner_tree @ BitcoinError::InvalidParam)]
    pub miner_tree: AccountLoader<'info, MinerTree>,

    /// Optional team account, required only when the owner is in a team.
    #[account(mut)]
    pub team: Option<Account<'info, Team>>,

    /// CHECK: blacklist marker PDA [SEED_BLACKLIST, owner]; validated in handler.
    pub blacklist: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn activate_miner(ctx: Context<Activate>) -> Result<()> {
    require_not_blacklisted(
        &ctx.accounts.blacklist.to_account_info(),
        ctx.program_id,
        &ctx.accounts.owner.key(),
    )?;

    let max_active_hr = ctx.accounts.config.max_active_hr;
    let miner = &mut ctx.accounts.miner_state;
    require!(!miner.active, BitcoinError::AlreadyActive);
    miner.owner = ctx.accounts.owner.key();
    let hr = miner.hashrate;

    let user_state = &mut ctx.accounts.user_state;
    if user_state.owner == Pubkey::default() {
        user_state.owner = ctx.accounts.owner.key();
        user_state.bump = ctx.bumps.user_state;
    }
    require!(
        user_state.active_hashrate.saturating_add(hr) <= max_active_hr,
        BitcoinError::HashrateCapExceeded
    );

    let slot = {
        let mut tree = ctx.accounts.miner_tree.load_mut()?;
        tree.insert(miner.nft_mint, hr)?
    };
    miner.tree_slot = slot;
    miner.active = true;
    miner.lock_until = 0;

    // Wallet-level team membership: inherit and contribute hashrate while active.
    miner.team = user_state.team;
    if user_state.has_team() {
        let team = ctx.accounts.team.as_mut().ok_or(BitcoinError::NotInTeam)?;
        require_keys_eq!(team.key(), user_state.team, BitcoinError::NotInTeam);
        team.total_active_hashrate = team
            .total_active_hashrate
            .checked_add(hr)
            .ok_or(BitcoinError::MathOverflow)?;
        miner.team_reward_debt = team.acc_reward_per_hashrate;
    }

    user_state.active_hashrate = user_state.active_hashrate.saturating_add(hr);
    user_state.active_count = user_state.active_count.saturating_add(1);

    emit!(MinerActivated {
        nft_mint: miner.nft_mint,
        owner: miner.owner,
        hashrate: hr,
        slot,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct Deactivate<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [SEED_USER, owner.key().as_ref()],
        bump = user_state.bump
    )]
    pub user_state: Account<'info, UserState>,

    #[account(
        mut,
        seeds = [SEED_MINER, nft_mint.key().as_ref()],
        bump = miner_state.bump,
        constraint = miner_state.owner == owner.key() @ BitcoinError::NotNftOwner
    )]
    pub miner_state: Account<'info, MinerState>,

    pub nft_mint: Account<'info, Mint>,

    #[account(mut, address = config.miner_tree @ BitcoinError::InvalidParam)]
    pub miner_tree: AccountLoader<'info, MinerTree>,

    /// Optional team account, required only when the miner is in a team.
    #[account(mut)]
    pub team: Option<Account<'info, Team>>,
}

pub fn deactivate_miner(ctx: Context<Deactivate>) -> Result<()> {
    let miner = &mut ctx.accounts.miner_state;
    require!(miner.active, BitcoinError::NotActive);
    let hr = miner.hashrate;

    if miner.has_team() {
        let team = ctx.accounts.team.as_mut().ok_or(BitcoinError::NotInTeam)?;
        require_keys_eq!(team.key(), miner.team, BitcoinError::NotInTeam);
        let owed = team
            .acc_reward_per_hashrate
            .checked_sub(miner.team_reward_debt)
            .ok_or(BitcoinError::MathOverflow)?
            .checked_mul(hr as u128)
            .ok_or(BitcoinError::MathOverflow)?
            / ACC_SCALE;
        miner.pending = miner
            .pending
            .checked_add(owed as u64)
            .ok_or(BitcoinError::MathOverflow)?;
        team.total_active_hashrate = team.total_active_hashrate.saturating_sub(hr);
        miner.team_reward_debt = team.acc_reward_per_hashrate;
    }

    {
        let mut tree = ctx.accounts.miner_tree.load_mut()?;
        tree.remove(miner.tree_slot, hr)?;
    }
    miner.tree_slot = 0;
    miner.active = false;

    let user_state = &mut ctx.accounts.user_state;
    user_state.active_hashrate = user_state.active_hashrate.saturating_sub(hr);
    user_state.active_count = user_state.active_count.saturating_sub(1);

    emit!(MinerDeactivated {
        nft_mint: miner.nft_mint,
        owner: miner.owner,
    });
    Ok(())
}