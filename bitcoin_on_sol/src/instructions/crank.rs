use anchor_lang::prelude::*;

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::events::{BlockCommitted, BlockWon};
use crate::state::{BlockRound, Config, Winner};
use crate::tree::MinerTree;
use crate::util::{expand_u64, read_randomness};

fn kind_meta(kind: u8) -> Result<(i64, u8)> {
    match kind {
        BLOCK_KIND_SMALL => Ok((SMALL_BLOCK_INTERVAL, SMALL_BLOCK_WINNERS)),
        BLOCK_KIND_BIG => Ok((BIG_BLOCK_INTERVAL, BIG_BLOCK_WINNERS)),
        _ => err!(BitcoinError::InvalidBlockKind),
    }
}

#[derive(Accounts)]
#[instruction(kind: u8, index: u64)]
pub struct CommitBlock<'info> {
    #[account(address = config.crank_authority @ BitcoinError::Unauthorized)]
    pub crank: Signer<'info>,

    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
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
    let (interval, winner_count) = kind_meta(kind)?;
    let now = Clock::get()?.unix_timestamp;
    let config = &mut ctx.accounts.config;

    let (expected_index, last_ts) = if kind == BLOCK_KIND_BIG {
        (config.big_block_index, config.last_big_ts)
    } else {
        (config.small_block_index, config.last_small_ts)
    };
    require!(index == expected_index, BitcoinError::InvalidParam);
    require!(now - last_ts >= interval, BitcoinError::BlockTooSoon);

    // Compute per-winner reward with halving, clamped so the whole block fits
    // within the remaining pool.
    let base = if kind == BLOCK_KIND_BIG {
        config.base_big_reward
    } else {
        config.base_small_reward
    };
    let halvings = if config.halving_interval == 0 {
        0
    } else {
        (config.cycle_index / config.halving_interval).min(63)
    };
    let mut per = base >> halvings;
    let max_per = config.pool_remaining / (winner_count as u64).max(1);
    if per > max_per {
        per = max_per;
    }

    let round = &mut ctx.accounts.block_round;
    round.kind = kind;
    round.index = index;
    round.randomness = ctx.accounts.randomness.key();
    round.commit_slot = Clock::get()?.slot;
    round.reward_each = per;
    round.winner_count = winner_count;
    round.winners = Vec::new();
    round.settled = false;
    round.bump = ctx.bumps.block_round;

    // Rate-limit the next commit of this kind.
    if kind == BLOCK_KIND_BIG {
        config.last_big_ts = now;
    } else {
        config.last_small_ts = now;
    }

    emit!(BlockCommitted {
        kind,
        index,
        reward_each: per,
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

    let per = ctx.accounts.block_round.reward_each;
    let want = ctx.accounts.block_round.winner_count as u64;
    let now = Clock::get()?.unix_timestamp;

    let mut winners: Vec<Winner> = Vec::new();
    {
        let tree = ctx.accounts.miner_tree.load()?;
        if tree.total > 0 {
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

    // Advance indices / cycle.
    if kind == BLOCK_KIND_BIG {
        config.big_block_index = config
            .big_block_index
            .checked_add(1)
            .ok_or(BitcoinError::MathOverflow)?;
        config.cycle_index = config
            .cycle_index
            .checked_add(1)
            .ok_or(BitcoinError::MathOverflow)?;
    } else {
        config.small_block_index = config
            .small_block_index
            .checked_add(1)
            .ok_or(BitcoinError::MathOverflow)?;
    }

    let round = &mut ctx.accounts.block_round;
    round.winners = winners;
    round.settled = true;
    Ok(())
}
