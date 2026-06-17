use anchor_lang::prelude::*;

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::events::{BlockCommitted, BlockWon};
use crate::state::{BlockRound, Config, Winner};
use crate::tree::MinerTree;
use crate::util::{expand_u64, read_randomness};

/// VRF expansion index used to derive the reward percentage (kept clear of the
/// winner-draw indices 0..winner_count).
const BPS_DRAW_INDEX: u64 = 1_000;

fn winners_for(kind: u8) -> Result<u8> {
    match kind {
        BLOCK_KIND_SMALL => Ok(SMALL_BLOCK_WINNERS),
        BLOCK_KIND_BIG => Ok(BIG_BLOCK_WINNERS),
        _ => err!(BitcoinError::InvalidBlockKind),
    }
}

#[derive(Accounts)]
#[instruction(kind: u8, index: u64)]
pub struct CommitBlock<'info> {
    #[account(address = config.crank_authority @ BitcoinError::Unauthorized)]
    pub crank: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_CONFIG],
        bump = config.bump,
        constraint = config.game_enabled @ BitcoinError::GameDisabled
    )]
    pub config: Account<'info, Config>,

    #[account(
        init,
        payer = payer,
        space = 8 + BlockRound::INIT_SPACE,
        seeds = [SEED_ROUND, &[kind], &index.to_le_bytes()],
        bump
    )]
    pub block_round: Account<'info, BlockRound>,

    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: Switchboard On-Demand randomness account (validated on settle).
    pub randomness: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn commit_block(ctx: Context<CommitBlock>, kind: u8, index: u64) -> Result<()> {
    require!(!ctx.accounts.config.paused, BitcoinError::Paused);
    let winner_count = winners_for(kind)?;
    let now = Clock::get()?.unix_timestamp;
    let config = &mut ctx.accounts.config;

    let (expected_index, last_ts, interval) = if kind == BLOCK_KIND_BIG {
        (config.big_block_index, config.last_big_ts, config.big_interval)
    } else {
        (config.small_block_index, config.last_small_ts, config.small_interval)
    };
    require!(index == expected_index, BitcoinError::InvalidParam);
    require!(now - last_ts >= interval, BitcoinError::BlockTooSoon);

    let round = &mut ctx.accounts.block_round;
    round.kind = kind;
    round.index = index;
    round.randomness = ctx.accounts.randomness.key();
    round.commit_slot = Clock::get()?.slot;
    round.reward_each = 0; // computed at settle from the revealed VRF
    round.winner_count = winner_count;
    round.winners = Vec::new();
    round.settled = false;
    round.bump = ctx.bumps.block_round;

    if kind == BLOCK_KIND_BIG {
        config.last_big_ts = now;
    } else {
        config.last_small_ts = now;
    }

    emit!(BlockCommitted {
        kind,
        index,
        reward_each: 0,
        timestamp: now,
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(kind: u8, index: u64)]
pub struct SettleBlock<'info> {
    #[account(address = config.crank_authority @ BitcoinError::Unauthorized)]
    pub crank: Signer<'info>,

    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [SEED_ROUND, &[kind], &index.to_le_bytes()],
        bump = block_round.bump,
        constraint = !block_round.settled @ BitcoinError::RoundAlreadySettled,
        constraint = block_round.kind == kind @ BitcoinError::InvalidBlockKind
    )]
    pub block_round: Account<'info, BlockRound>,

    /// CHECK: Validated against block_round.randomness in the handler.
    pub randomness: UncheckedAccount<'info>,

    #[account(address = config.miner_tree @ BitcoinError::InvalidParam)]
    pub miner_tree: AccountLoader<'info, MinerTree>,
}

pub fn settle_block(ctx: Context<SettleBlock>, kind: u8, _index: u64) -> Result<()> {
    let value = read_randomness(
        &ctx.accounts.randomness.to_account_info(),
        &ctx.accounts.block_round.randomness,
    )?;

    // Reward = random % (in bps) of emission_base, halved by total_blocks,
    // optionally multiplied, clamped to the remaining pool. All derived from the
    // revealed VRF, so the amount is verifiable.
    let config = &ctx.accounts.config;
    let (bps_min, bps_max) = if kind == BLOCK_KIND_BIG {
        (config.big_bps_min as u64, config.big_bps_max as u64)
    } else {
        (config.small_bps_min as u64, config.small_bps_max as u64)
    };
    let span = bps_max.saturating_sub(bps_min).saturating_add(1).max(1);
    let bps = bps_min + (expand_u64(&value, BPS_DRAW_INDEX) % span);

    let mut total = (config.emission_base as u128)
        .checked_mul(bps as u128)
        .ok_or(BitcoinError::MathOverflow)?
        / BPS_DENOM as u128;

    let halvings = if config.halving_interval == 0 {
        0
    } else {
        (config.total_blocks / config.halving_interval).min(63)
    };
    total >>= halvings;

    if config.multiplier_enabled {
        total = total
            .checked_mul(config.global_multiplier_bps as u128)
            .ok_or(BitcoinError::MathOverflow)?
            / BPS_DENOM as u128;
    }

    let pool = config.pool_remaining as u128;
    if total > pool {
        total = pool;
    }

    let winner_count = ctx.accounts.block_round.winner_count.max(1) as u128;
    let per = (total / winner_count) as u64;

    let want = ctx.accounts.block_round.winner_count as u64;
    let now = Clock::get()?.unix_timestamp;

    let mut winners: Vec<Winner> = Vec::new();
    {
        let tree = ctx.accounts.miner_tree.load()?;
        if tree.total > 0 && per > 0 {
            for i in 0..want {
                let r = expand_u64(&value, i) as u128;
                let target = r % (tree.total as u128);
                let slot = tree.find_by_prefix(target)?;
                let nft_mint = tree.mint_at(slot);
                winners.push(Winner {
                    nft_mint,
                    collected: false,
                });
                emit!(BlockWon {
                    kind,
                    index: ctx.accounts.block_round.index,
                    nft_mint,
                    reward: per,
                    timestamp: now,
                });
            }
        }
    }

    let payout_total = per
        .checked_mul(winners.len() as u64)
        .ok_or(BitcoinError::MathOverflow)?;

    let config = &mut ctx.accounts.config;
    config.pool_remaining = config
        .pool_remaining
        .checked_sub(payout_total)
        .ok_or(BitcoinError::PoolInsufficient)?;
    config.total_blocks = config
        .total_blocks
        .checked_add(1)
        .ok_or(BitcoinError::MathOverflow)?;

    if kind == BLOCK_KIND_BIG {
        config.big_block_index = config
            .big_block_index
            .checked_add(1)
            .ok_or(BitcoinError::MathOverflow)?;
        config.cycle_index = config.cycle_index.saturating_add(1);
    } else {
        config.small_block_index = config
            .small_block_index
            .checked_add(1)
            .ok_or(BitcoinError::MathOverflow)?;
    }

    let round = &mut ctx.accounts.block_round;
    round.reward_each = per;
    round.winners = winners;
    round.settled = true;
    Ok(())
}