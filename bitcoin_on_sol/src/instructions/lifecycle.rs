use anchor_lang::prelude::*;
use anchor_spl::metadata::{
    freeze_delegated_account, thaw_delegated_account, FreezeDelegatedAccount, Metadata,
    ThawDelegatedAccount,
};
use anchor_spl::token::{approve, revoke, Approve, Mint, Revoke, Token, TokenAccount};

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::events::{MinerActivated, MinerDeactivated};
use crate::state::{Config, MinerState, Team, UserState};
use crate::tree::MinerTree;

#[derive(Accounts)]
pub struct Activate<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [SEED_CONFIG],
        bump = config.bump,
        constraint = !config.paused @ BitcoinError::Paused
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

    #[account(
        mut,
        constraint = nft_token.mint == nft_mint.key() @ BitcoinError::MintMismatch,
        constraint = nft_token.owner == owner.key() @ BitcoinError::NotNftOwner,
        constraint = nft_token.amount == 1 @ BitcoinError::NotNftOwner
    )]
    pub nft_token: Account<'info, TokenAccount>,

    /// CHECK: NFT metadata account, validated by the Token Metadata program.
    #[account(mut)]
    pub metadata: UncheckedAccount<'info>,

    /// CHECK: PDA delegate + (via edition) freeze authority.
    #[account(seeds = [SEED_MINT_AUTH], bump = config.mint_auth_bump)]
    pub mint_authority: UncheckedAccount<'info>,

    /// CHECK: Master edition (freeze authority holder), validated by Token Metadata.
    #[account(mut)]
    pub master_edition: UncheckedAccount<'info>,

    #[account(mut, address = config.miner_tree @ BitcoinError::InvalidParam)]
    pub miner_tree: AccountLoader<'info, MinerTree>,

    /// Optional team account, required only when the miner is in a team.
    #[account(mut)]
    pub team: Option<Account<'info, Team>>,

    pub token_program: Program<'info, Token>,
    pub token_metadata_program: Program<'info, Metadata>,
    pub system_program: Program<'info, System>,
}

pub fn activate_miner(ctx: Context<Activate>) -> Result<()> {
    let miner = &mut ctx.accounts.miner_state;
    require!(!miner.active, BitcoinError::AlreadyActive);

    // Refresh ownership from the token account (handles post-mint transfers).
    miner.owner = ctx.accounts.owner.key();

    let user_state = &mut ctx.accounts.user_state;
    if user_state.owner == Pubkey::default() {
        user_state.owner = ctx.accounts.owner.key();
        user_state.bump = ctx.bumps.user_state;
    }
    require!(
        user_state.active_count < MAX_ACTIVE_PER_USER,
        BitcoinError::TooManyActiveMiners
    );

    // Approve PDA as delegate of the single NFT, then freeze it.
    approve(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Approve {
                to: ctx.accounts.nft_token.to_account_info(),
                delegate: ctx.accounts.mint_authority.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        1,
    )?;

    let mint_seeds: &[&[u8]] = &[SEED_MINT_AUTH, &[ctx.accounts.config.mint_auth_bump]];
    let signer = &[mint_seeds];
    freeze_delegated_account(CpiContext::new_with_signer(
        ctx.accounts.token_metadata_program.to_account_info(),
        FreezeDelegatedAccount {
            metadata: ctx.accounts.metadata.to_account_info(),
            delegate: ctx.accounts.mint_authority.to_account_info(),
            token_account: ctx.accounts.nft_token.to_account_info(),
            edition: ctx.accounts.master_edition.to_account_info(),
            mint: ctx.accounts.nft_mint.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        },
        signer,
    ))?;

    // Insert into the lottery tree.
    let slot = {
        let mut tree = ctx.accounts.miner_tree.load_mut()?;
        tree.insert(miner.nft_mint, miner.hashrate)?
    };
    miner.tree_slot = slot;
    miner.active = true;
    miner.lock_until = Clock::get()?
        .unix_timestamp
        .checked_add(ACTIVATION_LOCK_SECONDS)
        .ok_or(BitcoinError::MathOverflow)?;

    // Membership is wallet-level: this miner inherits its owner's current team
    // and contributes its hashrate to that team's pool while active.
    miner.team = user_state.team;
    if user_state.has_team() {
        let team = ctx
            .accounts
            .team
            .as_mut()
            .ok_or(BitcoinError::NotInTeam)?;
        require_keys_eq!(team.key(), user_state.team, BitcoinError::NotInTeam);
        team.total_active_hashrate = team
            .total_active_hashrate
            .checked_add(miner.hashrate)
            .ok_or(BitcoinError::MathOverflow)?;
        miner.team_reward_debt = team.acc_reward_per_hashrate;
    }

    user_state.active_count = user_state
        .active_count
        .checked_add(1)
        .ok_or(BitcoinError::MathOverflow)?;

    emit!(MinerActivated {
        nft_mint: miner.nft_mint,
        owner: miner.owner,
        hashrate: miner.hashrate,
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

    #[account(
        mut,
        constraint = nft_token.mint == nft_mint.key() @ BitcoinError::MintMismatch,
        constraint = nft_token.owner == owner.key() @ BitcoinError::NotNftOwner
    )]
    pub nft_token: Account<'info, TokenAccount>,

    /// CHECK: NFT metadata account, validated by the Token Metadata program.
    #[account(mut)]
    pub metadata: UncheckedAccount<'info>,

    /// CHECK: PDA delegate + (via edition) freeze authority.
    #[account(seeds = [SEED_MINT_AUTH], bump = config.mint_auth_bump)]
    pub mint_authority: UncheckedAccount<'info>,

    /// CHECK: Master edition (freeze authority holder), validated by Token Metadata.
    #[account(mut)]
    pub master_edition: UncheckedAccount<'info>,

    #[account(mut, address = config.miner_tree @ BitcoinError::InvalidParam)]
    pub miner_tree: AccountLoader<'info, MinerTree>,

    /// Optional team account, required only when the miner is in a team.
    #[account(mut)]
    pub team: Option<Account<'info, Team>>,

    pub token_program: Program<'info, Token>,
    pub token_metadata_program: Program<'info, Metadata>,
}

pub fn deactivate_miner(ctx: Context<Deactivate>) -> Result<()> {
    let miner = &mut ctx.accounts.miner_state;
    require!(miner.active, BitcoinError::NotActive);
    // 12-hour activation lock: cannot deactivate (or therefore sacrifice) early.
    require!(
        Clock::get()?.unix_timestamp >= miner.lock_until,
        BitcoinError::MinerLocked
    );

    // Realize any team earnings, then remove this miner's hashrate from the team.
    if miner.has_team() {
        let team = ctx
            .accounts
            .team
            .as_mut()
            .ok_or(BitcoinError::NotInTeam)?;
        require_keys_eq!(team.key(), miner.team, BitcoinError::NotInTeam);
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
        miner.team_reward_debt = team.acc_reward_per_hashrate;
    }

    // Remove from the lottery tree.
    {
        let mut tree = ctx.accounts.miner_tree.load_mut()?;
        tree.remove(miner.tree_slot, miner.hashrate)?;
    }
    miner.tree_slot = 0;
    miner.active = false;

    // Thaw and revoke delegate.
    let mint_seeds: &[&[u8]] = &[SEED_MINT_AUTH, &[ctx.accounts.config.mint_auth_bump]];
    let signer = &[mint_seeds];
    thaw_delegated_account(CpiContext::new_with_signer(
        ctx.accounts.token_metadata_program.to_account_info(),
        ThawDelegatedAccount {
            metadata: ctx.accounts.metadata.to_account_info(),
            delegate: ctx.accounts.mint_authority.to_account_info(),
            token_account: ctx.accounts.nft_token.to_account_info(),
            edition: ctx.accounts.master_edition.to_account_info(),
            mint: ctx.accounts.nft_mint.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        },
        signer,
    ))?;
    revoke(CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        Revoke {
            source: ctx.accounts.nft_token.to_account_info(),
            authority: ctx.accounts.owner.to_account_info(),
        },
    ))?;

    let user_state = &mut ctx.accounts.user_state;
    user_state.active_count = user_state.active_count.saturating_sub(1);

    emit!(MinerDeactivated {
        nft_mint: miner.nft_mint,
        owner: miner.owner,
    });
    Ok(())
}
