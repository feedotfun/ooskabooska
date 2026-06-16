use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::mpl_token_metadata::types::{Collection, Creator, DataV2};
use anchor_spl::metadata::{
    burn_nft, create_master_edition_v3, create_metadata_accounts_v3, verify_sized_collection_item,
    BurnNft, CreateMasterEditionV3, CreateMetadataAccountsV3, Metadata, VerifySizedCollectionItem,
};
use anchor_spl::token::{burn, mint_to, Burn, Mint, MintTo, Token, TokenAccount};

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::events::Sacrificed;
use crate::instructions::mint::NFT_SYMBOL;
use crate::state::{Config, MinerState};

#[derive(Accounts)]
pub struct Sacrifice<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_CONFIG],
        bump = config.bump,
        constraint = !config.paused @ BitcoinError::Paused
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        address = config.token_mint @ BitcoinError::MintMismatch
    )]
    pub token_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_token.mint == config.token_mint @ BitcoinError::MintMismatch,
        constraint = user_token.owner == owner.key() @ BitcoinError::OwnerMismatch
    )]
    pub user_token: Account<'info, TokenAccount>,

    /// CHECK: PDA mint/update authority.
    #[account(seeds = [SEED_MINT_AUTH], bump = config.mint_auth_bump)]
    pub mint_authority: UncheckedAccount<'info>,

    #[account(address = config.collection_mint @ BitcoinError::MintMismatch)]
    pub collection_mint: Account<'info, Mint>,
    /// CHECK: Collection metadata (size updated by Token Metadata).
    #[account(mut)]
    pub collection_metadata: UncheckedAccount<'info>,
    /// CHECK: Collection master edition.
    pub collection_master_edition: UncheckedAccount<'info>,

    // ---- Sacrificed NFT A ----
    #[account(mut)]
    pub mint_a: Account<'info, Mint>,
    #[account(
        mut,
        constraint = token_a.mint == mint_a.key() @ BitcoinError::MintMismatch,
        constraint = token_a.owner == owner.key() @ BitcoinError::NotNftOwner,
        constraint = token_a.amount == 1 @ BitcoinError::NotNftOwner
    )]
    pub token_a: Account<'info, TokenAccount>,
    /// CHECK: Metadata of A.
    #[account(mut)]
    pub metadata_a: UncheckedAccount<'info>,
    /// CHECK: Master edition of A.
    #[account(mut)]
    pub edition_a: UncheckedAccount<'info>,
    #[account(
        mut,
        close = owner,
        seeds = [SEED_MINER, mint_a.key().as_ref()],
        bump = miner_a.bump,
        constraint = miner_a.owner == owner.key() @ BitcoinError::NotNftOwner,
        constraint = !miner_a.active @ BitcoinError::MustBeInactive
    )]
    pub miner_a: Account<'info, MinerState>,

    // ---- Sacrificed NFT B ----
    #[account(mut)]
    pub mint_b: Account<'info, Mint>,
    #[account(
        mut,
        constraint = token_b.mint == mint_b.key() @ BitcoinError::MintMismatch,
        constraint = token_b.owner == owner.key() @ BitcoinError::NotNftOwner,
        constraint = token_b.amount == 1 @ BitcoinError::NotNftOwner
    )]
    pub token_b: Account<'info, TokenAccount>,
    /// CHECK: Metadata of B.
    #[account(mut)]
    pub metadata_b: UncheckedAccount<'info>,
    /// CHECK: Master edition of B.
    #[account(mut)]
    pub edition_b: UncheckedAccount<'info>,
    #[account(
        mut,
        close = owner,
        seeds = [SEED_MINER, mint_b.key().as_ref()],
        bump = miner_b.bump,
        constraint = miner_b.owner == owner.key() @ BitcoinError::NotNftOwner,
        constraint = !miner_b.active @ BitcoinError::MustBeInactive
    )]
    pub miner_b: Account<'info, MinerState>,

    // ---- Forged NFT ----
    #[account(
        init,
        payer = owner,
        mint::decimals = 0,
        mint::authority = mint_authority,
        mint::freeze_authority = mint_authority
    )]
    pub new_mint: Account<'info, Mint>,
    #[account(
        init,
        payer = owner,
        associated_token::mint = new_mint,
        associated_token::authority = owner
    )]
    pub new_token: Account<'info, TokenAccount>,
    /// CHECK: Created by Token Metadata CPI.
    #[account(mut)]
    pub new_metadata: UncheckedAccount<'info>,
    /// CHECK: Created by Token Metadata CPI.
    #[account(mut)]
    pub new_master_edition: UncheckedAccount<'info>,
    #[account(
        init,
        payer = owner,
        space = 8 + MinerState::INIT_SPACE,
        seeds = [SEED_MINER, new_mint.key().as_ref()],
        bump
    )]
    pub new_miner: Account<'info, MinerState>,

    pub token_metadata_program: Program<'info, Metadata>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn sacrifice(ctx: Context<Sacrifice>, name: String, uri: String) -> Result<()> {
    // Validate the pair.
    require_keys_neq!(
        ctx.accounts.mint_a.key(),
        ctx.accounts.mint_b.key(),
        BitcoinError::DuplicateNft
    );
    let from_tier = ctx.accounts.miner_a.tier;
    require!(
        ctx.accounts.miner_b.tier == from_tier,
        BitcoinError::TierMismatch
    );
    require!(from_tier < TIER_GRAIL, BitcoinError::GrailNotForgeable);
    let to_tier = from_tier + 1;
    require!(
        to_tier <= MAX_SACRIFICE_RESULT_TIER,
        BitcoinError::GrailNotForgeable
    );

    // Burn the upgrade cost in tokens.
    let cost = ctx.accounts.config.upgrade_cost[from_tier as usize];
    if cost > 0 {
        burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.token_mint.to_account_info(),
                    from: ctx.accounts.user_token.to_account_info(),
                    authority: ctx.accounts.owner.to_account_info(),
                },
            ),
            cost,
        )?;
        let config = &mut ctx.accounts.config;
        config.total_burned = config
            .total_burned
            .checked_add(cost)
            .ok_or(BitcoinError::MathOverflow)?;
    }

    // Burn both sacrificed NFTs (also decrements the sized collection).
    let collection_metadata_key = ctx.accounts.collection_metadata.key();
    burn_nft(
        CpiContext::new(
            ctx.accounts.token_metadata_program.to_account_info(),
            BurnNft {
                metadata: ctx.accounts.metadata_a.to_account_info(),
                owner: ctx.accounts.owner.to_account_info(),
                mint: ctx.accounts.mint_a.to_account_info(),
                token: ctx.accounts.token_a.to_account_info(),
                edition: ctx.accounts.edition_a.to_account_info(),
                spl_token: ctx.accounts.token_program.to_account_info(),
            },
        )
        .with_remaining_accounts(vec![ctx.accounts.collection_metadata.to_account_info()]),
        Some(collection_metadata_key),
    )?;
    burn_nft(
        CpiContext::new(
            ctx.accounts.token_metadata_program.to_account_info(),
            BurnNft {
                metadata: ctx.accounts.metadata_b.to_account_info(),
                owner: ctx.accounts.owner.to_account_info(),
                mint: ctx.accounts.mint_b.to_account_info(),
                token: ctx.accounts.token_b.to_account_info(),
                edition: ctx.accounts.edition_b.to_account_info(),
                spl_token: ctx.accounts.token_program.to_account_info(),
            },
        )
        .with_remaining_accounts(vec![ctx.accounts.collection_metadata.to_account_info()]),
        Some(collection_metadata_key),
    )?;

    // Forge the upgraded NFT.
    let mint_auth_bump = ctx.accounts.config.mint_auth_bump;
    let mint_seeds: &[&[u8]] = &[SEED_MINT_AUTH, &[mint_auth_bump]];
    let signer = &[mint_seeds];

    mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.new_mint.to_account_info(),
                to: ctx.accounts.new_token.to_account_info(),
                authority: ctx.accounts.mint_authority.to_account_info(),
            },
            signer,
        ),
        1,
    )?;

    let creators = vec![Creator {
        address: ctx.accounts.mint_authority.key(),
        verified: true,
        share: 100,
    }];

    create_metadata_accounts_v3(
        CpiContext::new_with_signer(
            ctx.accounts.token_metadata_program.to_account_info(),
            CreateMetadataAccountsV3 {
                metadata: ctx.accounts.new_metadata.to_account_info(),
                mint: ctx.accounts.new_mint.to_account_info(),
                mint_authority: ctx.accounts.mint_authority.to_account_info(),
                update_authority: ctx.accounts.mint_authority.to_account_info(),
                payer: ctx.accounts.owner.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
                rent: ctx.accounts.rent.to_account_info(),
            },
            signer,
        ),
        DataV2 {
            name,
            symbol: NFT_SYMBOL.to_string(),
            uri,
            seller_fee_basis_points: 0,
            creators: Some(creators),
            collection: Some(Collection {
                verified: false,
                key: ctx.accounts.collection_mint.key(),
            }),
            uses: None,
        },
        true,
        true,
        None,
    )?;

    create_master_edition_v3(
        CpiContext::new_with_signer(
            ctx.accounts.token_metadata_program.to_account_info(),
            CreateMasterEditionV3 {
                edition: ctx.accounts.new_master_edition.to_account_info(),
                mint: ctx.accounts.new_mint.to_account_info(),
                update_authority: ctx.accounts.mint_authority.to_account_info(),
                mint_authority: ctx.accounts.mint_authority.to_account_info(),
                payer: ctx.accounts.owner.to_account_info(),
                metadata: ctx.accounts.new_metadata.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
                rent: ctx.accounts.rent.to_account_info(),
            },
            signer,
        ),
        Some(0),
    )?;

    verify_sized_collection_item(
        CpiContext::new_with_signer(
            ctx.accounts.token_metadata_program.to_account_info(),
            VerifySizedCollectionItem {
                payer: ctx.accounts.owner.to_account_info(),
                metadata: ctx.accounts.new_metadata.to_account_info(),
                collection_authority: ctx.accounts.mint_authority.to_account_info(),
                collection_mint: ctx.accounts.collection_mint.to_account_info(),
                collection_metadata: ctx.accounts.collection_metadata.to_account_info(),
                collection_master_edition: ctx
                    .accounts
                    .collection_master_edition
                    .to_account_info(),
            },
            signer,
        ),
        None,
    )?;

    let miner = &mut ctx.accounts.new_miner;
    miner.owner = ctx.accounts.owner.key();
    miner.nft_mint = ctx.accounts.new_mint.key();
    miner.tier = to_tier;
    miner.hashrate = TIER_HASHRATE[to_tier as usize];
    miner.active = false;
    miner.tree_slot = 0;
    miner.team = Pubkey::default();
    miner.team_reward_debt = 0;
    miner.pending = 0;
    miner.blocks_won = 0;
    miner.total_earned = 0;
    miner.created_at = Clock::get()?.unix_timestamp;
    miner.lock_until = 0;
    miner.bump = ctx.bumps.new_miner;

    emit!(Sacrificed {
        owner: ctx.accounts.owner.key(),
        from_tier,
        to_tier,
        burned_a: ctx.accounts.mint_a.key(),
        burned_b: ctx.accounts.mint_b.key(),
        new_mint: ctx.accounts.new_mint.key(),
    });
    Ok(())
}
