use anchor_lang::prelude::*;
use anchor_spl::token::{transfer, Mint, Token, TokenAccount, Transfer};

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::events::{RewardsClaimed, WinCollected};
use crate::state::{Config, MinerState, Team};

#[derive(Accounts)]
#[instruction(kind: u8, index: u64, winner_index: u8)]
pub struct CollectWin<'info> {
    pub caller: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [SEED_ROUND, &[kind], &index.to_le_bytes()],
        bump = block_round.bump,
        constraint = block_round.settled @ BitcoinError::RoundNotCommitted
    )]
    pub block_round: Account<'info, crate::state::BlockRound>,

    pub nft_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [SEED_MINER, nft_mint.key().as_ref()],
        bump = miner_state.bump
    )]
    pub miner_state: Account<'info, MinerState>,

    /// Optional team account, required only when the winning miner is teamed.
    #[account(mut)]
    pub team: Option<Account<'info, Team>>,
}

pub fn collect_win(
    ctx: Context<CollectWin>,
    kind: u8,
    _index: u64,
    winner_index: u8,
) -> Result<()> {
    require!(kind == ctx.accounts.block_round.kind, BitcoinError::InvalidBlockKind);
    let round = &mut ctx.accounts.block_round;
    let wi = winner_index as usize;
    require!(wi < round.winners.len(), BitcoinError::NotAWinner);

    let reward = round.reward_each;
    {
        let w = &mut round.winners[wi];
        require_keys_eq!(
            w.nft_mint,
            ctx.accounts.nft_mint.key(),
            BitcoinError::WinnerMismatch
        );
        require!(!w.collected, BitcoinError::AlreadyCollected);
        w.collected = true;
    }

    let miner = &mut ctx.accounts.miner_state;
    miner.blocks_won = miner.blocks_won.checked_add(1).ok_or(BitcoinError::MathOverflow)?;
    miner.total_earned = miner
        .total_earned
        .checked_add(reward)
        .ok_or(BitcoinError::MathOverflow)?;

    let to_team = miner.has_team();
    if to_team {
        let team = ctx.accounts.team.as_mut().ok_or(BitcoinError::NotInTeam)?;
        require_keys_eq!(team.key(), miner.team, BitcoinError::NotInTeam);
        require!(
            team.total_active_hashrate > 0,
            BitcoinError::NoActiveMiners
        );
        // Distribute proportionally across the team's active hashrate.
        let add = (reward as u128)
            .checked_mul(ACC_SCALE)
            .ok_or(BitcoinError::MathOverflow)?
            / team.total_active_hashrate as u128;
        team.acc_reward_per_hashrate = team
            .acc_reward_per_hashrate
            .checked_add(add)
            .ok_or(BitcoinError::MathOverflow)?;
    } else {
        miner.pending = miner
            .pending
            .checked_add(reward)
            .ok_or(BitcoinError::MathOverflow)?;
    }

    emit!(WinCollected {
        kind,
        index: round.index,
        nft_mint: miner.nft_mint,
        reward,
        to_team,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct ClaimRewards<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [SEED_MINER, miner_state.nft_mint.as_ref()],
        bump = miner_state.bump,
        constraint = miner_state.owner == owner.key() @ BitcoinError::NotNftOwner,
        constraint = !miner_state.has_team() @ BitcoinError::MustLeaveTeam
    )]
    pub miner_state: Account<'info, MinerState>,

    #[account(mut, address = config.reward_vault @ BitcoinError::InvalidParam)]
    pub reward_vault: Account<'info, TokenAccount>,

    /// CHECK: PDA authority over the reward vault.
    #[account(seeds = [SEED_VAULT_AUTH], bump = config.vault_auth_bump)]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = owner_token.mint == config.token_mint @ BitcoinError::MintMismatch,
        constraint = owner_token.owner == owner.key() @ BitcoinError::OwnerMismatch
    )]
    pub owner_token: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub fn claim_rewards(ctx: Context<ClaimRewards>) -> Result<()> {
    let amount = ctx.accounts.miner_state.pending;
    require!(amount > 0, BitcoinError::NothingToClaim);
    ctx.accounts.miner_state.pending = 0;
    pay_from_vault(
        amount,
        &ctx.accounts.config,
        &ctx.accounts.reward_vault,
        &ctx.accounts.vault_authority,
        &ctx.accounts.owner_token,
        &ctx.accounts.token_program,
    )?;
    emit!(RewardsClaimed {
        owner: ctx.accounts.owner.key(),
        amount,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct ClaimTeamRewards<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [SEED_MINER, miner_state.nft_mint.as_ref()],
        bump = miner_state.bump,
        constraint = miner_state.owner == owner.key() @ BitcoinError::NotNftOwner
    )]
    pub miner_state: Account<'info, MinerState>,

    #[account(
        constraint = miner_state.team == team.key() @ BitcoinError::NotInTeam
    )]
    pub team: Account<'info, Team>,

    #[account(mut, address = config.reward_vault @ BitcoinError::InvalidParam)]
    pub reward_vault: Account<'info, TokenAccount>,

    /// CHECK: PDA authority over the reward vault.
    #[account(seeds = [SEED_VAULT_AUTH], bump = config.vault_auth_bump)]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = owner_token.mint == config.token_mint @ BitcoinError::MintMismatch,
        constraint = owner_token.owner == owner.key() @ BitcoinError::OwnerMismatch
    )]
    pub owner_token: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub fn claim_team_rewards(ctx: Context<ClaimTeamRewards>) -> Result<()> {
    let miner = &mut ctx.accounts.miner_state;
    let team = &ctx.accounts.team;

    // Realize accrued team share for an active member up to the current accumulator.
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
        miner.team_reward_debt = team.acc_reward_per_hashrate;
    }

    let amount = miner.pending;
    require!(amount > 0, BitcoinError::NothingToClaim);
    miner.pending = 0;

    pay_from_vault(
        amount,
        &ctx.accounts.config,
        &ctx.accounts.reward_vault,
        &ctx.accounts.vault_authority,
        &ctx.accounts.owner_token,
        &ctx.accounts.token_program,
    )?;
    emit!(RewardsClaimed {
        owner: ctx.accounts.owner.key(),
        amount,
    });
    Ok(())
}

fn pay_from_vault<'info>(
    amount: u64,
    config: &Account<'info, Config>,
    reward_vault: &Account<'info, TokenAccount>,
    vault_authority: &UncheckedAccount<'info>,
    owner_token: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    require!(reward_vault.amount >= amount, BitcoinError::PoolInsufficient);
    let seeds: &[&[u8]] = &[SEED_VAULT_AUTH, &[config.vault_auth_bump]];
    let signer = &[seeds];
    transfer(
        CpiContext::new_with_signer(
            token_program.to_account_info(),
            Transfer {
                from: reward_vault.to_account_info(),
                to: owner_token.to_account_info(),
                authority: vault_authority.to_account_info(),
            },
            signer,
        ),
        amount,
    )?;
    Ok(())
}
