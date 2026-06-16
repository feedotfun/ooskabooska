use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::mpl_token_metadata::types::{CollectionDetails, Creator, DataV2};
use anchor_spl::metadata::{
    create_master_edition_v3, create_metadata_accounts_v3, CreateMasterEditionV3,
    CreateMetadataAccountsV3, Metadata,
};
use anchor_spl::token::{mint_to, Mint, MintTo, Token, TokenAccount, Transfer, transfer};

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::state::Config;
use crate::tree::MinerTree;

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitConfigParams {
    pub crank_authority: Pubkey,
    pub switchboard_queue: Pubkey,
    pub mint_price: u64,
    pub upgrade_cost: [u64; 4],
    pub base_small_reward: u64,
    pub base_big_reward: u64,
    pub halving_interval: u64,
}

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        init,
        payer = admin,
        space = 8 + Config::INIT_SPACE,
        seeds = [SEED_CONFIG],
        bump
    )]
    pub config: Account<'info, Config>,

    pub token_mint: Account<'info, Mint>,

    /// CHECK: PDA authority that owns the reward vault. Not deserialized.
    #[account(seeds = [SEED_VAULT_AUTH], bump)]
    pub vault_authority: UncheckedAccount<'info>,

    /// CHECK: PDA authority for NFT mints / collection. Not deserialized.
    #[account(seeds = [SEED_MINT_AUTH], bump)]
    pub mint_authority: UncheckedAccount<'info>,

    #[account(
        init,
        payer = admin,
        associated_token::mint = token_mint,
        associated_token::authority = vault_authority
    )]
    pub reward_vault: Account<'info, TokenAccount>,

    /// The lottery tree (~112 KB) is too large for a CPI `init`, so it is
    /// created client-side (System `createAccount`, owned by this program) and
    /// initialized here via the `zero` constraint.
    #[account(zero)]
    pub miner_tree: AccountLoader<'info, MinerTree>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn initialize_config(ctx: Context<InitializeConfig>, params: InitConfigParams) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.admin = ctx.accounts.admin.key();
    config.crank_authority = params.crank_authority;
    config.token_mint = ctx.accounts.token_mint.key();
    config.reward_vault = ctx.accounts.reward_vault.key();
    config.collection_mint = Pubkey::default(); // set in create_collection
    config.switchboard_queue = params.switchboard_queue;
    config.miner_tree = ctx.accounts.miner_tree.key();
    config.mint_price = params.mint_price;
    config.upgrade_cost = params.upgrade_cost;
    config.base_small_reward = params.base_small_reward;
    config.base_big_reward = params.base_big_reward;
    config.halving_interval = params.halving_interval;
    config.total_burned = 0;
    config.pool_remaining = 0;
    config.cycle_index = 0;
    config.small_block_index = 0;
    config.big_block_index = 0;
    config.last_small_ts = 0;
    config.last_big_ts = 0;
    config.tier_remaining = INITIAL_TIER_REMAINING;
    config.minted_total = 0;
    config.team_creation_fee_lamports = DEFAULT_TEAM_CREATION_FEE_LAMPORTS;
    config.max_team_members = DEFAULT_MAX_TEAM_MEMBERS;
    config.teams_enabled = true;
    config.paused = false;
    config.bump = ctx.bumps.config;
    config.vault_auth_bump = ctx.bumps.vault_authority;
    config.mint_auth_bump = ctx.bumps.mint_authority;

    let mut tree = ctx.accounts.miner_tree.load_init()?;
    tree.initialize();
    Ok(())
}

#[derive(Accounts)]
pub struct CreateCollection<'info> {
    #[account(mut, address = config.admin @ BitcoinError::Unauthorized)]
    pub admin: Signer<'info>,

    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    /// CHECK: PDA signer / update + mint authority for the collection.
    #[account(seeds = [SEED_MINT_AUTH], bump = config.mint_auth_bump)]
    pub mint_authority: UncheckedAccount<'info>,

    #[account(
        init,
        payer = admin,
        mint::decimals = 0,
        mint::authority = mint_authority,
        mint::freeze_authority = mint_authority
    )]
    pub collection_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = admin,
        associated_token::mint = collection_mint,
        associated_token::authority = admin
    )]
    pub collection_token: Account<'info, TokenAccount>,

    /// CHECK: Created via Token Metadata CPI.
    #[account(mut)]
    pub metadata: UncheckedAccount<'info>,
    /// CHECK: Created via Token Metadata CPI.
    #[account(mut)]
    pub master_edition: UncheckedAccount<'info>,

    pub token_metadata_program: Program<'info, Metadata>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn create_collection(
    ctx: Context<CreateCollection>,
    name: String,
    symbol: String,
    uri: String,
) -> Result<()> {
    let mint_seeds: &[&[u8]] = &[SEED_MINT_AUTH, &[ctx.accounts.config.mint_auth_bump]];
    let signer = &[mint_seeds];

    // Mint exactly one collection token to the admin's ATA.
    mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.collection_mint.to_account_info(),
                to: ctx.accounts.collection_token.to_account_info(),
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
                metadata: ctx.accounts.metadata.to_account_info(),
                mint: ctx.accounts.collection_mint.to_account_info(),
                mint_authority: ctx.accounts.mint_authority.to_account_info(),
                update_authority: ctx.accounts.mint_authority.to_account_info(),
                payer: ctx.accounts.admin.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
                rent: ctx.accounts.rent.to_account_info(),
            },
            signer,
        ),
        DataV2 {
            name,
            symbol,
            uri,
            seller_fee_basis_points: 0,
            creators: Some(creators),
            collection: None,
            uses: None,
        },
        true,
        true,
        Some(CollectionDetails::V1 { size: 0 }),
    )?;

    create_master_edition_v3(
        CpiContext::new_with_signer(
            ctx.accounts.token_metadata_program.to_account_info(),
            CreateMasterEditionV3 {
                edition: ctx.accounts.master_edition.to_account_info(),
                mint: ctx.accounts.collection_mint.to_account_info(),
                update_authority: ctx.accounts.mint_authority.to_account_info(),
                mint_authority: ctx.accounts.mint_authority.to_account_info(),
                payer: ctx.accounts.admin.to_account_info(),
                metadata: ctx.accounts.metadata.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
                rent: ctx.accounts.rent.to_account_info(),
            },
            signer,
        ),
        Some(0),
    )?;

    ctx.accounts.config.collection_mint = ctx.accounts.collection_mint.key();
    Ok(())
}

#[derive(Accounts)]
pub struct FundRewardPool<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,

    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        address = config.reward_vault @ BitcoinError::InvalidParam
    )]
    pub reward_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = funder_token.mint == config.token_mint @ BitcoinError::MintMismatch,
        constraint = funder_token.owner == funder.key() @ BitcoinError::OwnerMismatch
    )]
    pub funder_token: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub fn fund_reward_pool(ctx: Context<FundRewardPool>, amount: u64) -> Result<()> {
    require!(amount > 0, BitcoinError::InvalidParam);
    transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.funder_token.to_account_info(),
                to: ctx.accounts.reward_vault.to_account_info(),
                authority: ctx.accounts.funder.to_account_info(),
            },
        ),
        amount,
    )?;
    let config = &mut ctx.accounts.config;
    config.pool_remaining = config
        .pool_remaining
        .checked_add(amount)
        .ok_or(BitcoinError::MathOverflow)?;
    Ok(())
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    #[account(address = config.admin @ BitcoinError::Unauthorized)]
    pub admin: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
}

pub fn set_prices(
    ctx: Context<AdminOnly>,
    mint_price: u64,
    upgrade_cost: [u64; 4],
) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.mint_price = mint_price;
    config.upgrade_cost = upgrade_cost;
    Ok(())
}

pub fn set_emission(
    ctx: Context<AdminOnly>,
    base_small_reward: u64,
    base_big_reward: u64,
    halving_interval: u64,
) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.base_small_reward = base_small_reward;
    config.base_big_reward = base_big_reward;
    config.halving_interval = halving_interval;
    Ok(())
}

pub fn set_crank_authority(ctx: Context<AdminOnly>, new_authority: Pubkey) -> Result<()> {
    ctx.accounts.config.crank_authority = new_authority;
    Ok(())
}

pub fn set_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
    ctx.accounts.config.paused = paused;
    Ok(())
}

/// Configure team rules: creation fee, member cap, and whether creation is open.
pub fn set_team_params(
    ctx: Context<AdminOnly>,
    creation_fee_lamports: u64,
    max_members: u8,
    teams_enabled: bool,
) -> Result<()> {
    require!(max_members >= 1, BitcoinError::InvalidParam);
    let config = &mut ctx.accounts.config;
    config.team_creation_fee_lamports = creation_fee_lamports;
    config.max_team_members = max_members;
    config.teams_enabled = teams_enabled;
    Ok(())
}
