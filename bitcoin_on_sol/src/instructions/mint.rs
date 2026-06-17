use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::mpl_token_metadata::types::{Collection, Creator, DataV2};
use anchor_spl::metadata::{
    create_master_edition_v3, create_metadata_accounts_v3, verify_sized_collection_item,
    CreateMasterEditionV3, CreateMetadataAccountsV3, Metadata, VerifySizedCollectionItem,
};
use anchor_spl::token::{burn, mint_to, Burn, Mint, MintTo, Token, TokenAccount};

use crate::constants::*;
use crate::errors::BitcoinError;
use crate::events::{MintRequested, MintRevealed};
use crate::state::{Config, MinerState, PendingMint, Team, UserState};
use crate::tree::MinerTree;
use crate::util::{expand_u64, read_randomness, require_not_blacklisted};

pub const NFT_SYMBOL: &str = "BOS";

// ---------------------------------------------------------------------------
// dev_mint: an instant, single-instruction mint used while the full VRF +
// Metaplex flow is being finalized. It creates a "temporary" on-chain miner
// (no SPL/Metaplex token yet) carrying its tier, hashpower and mining status so
// the UI can show real, owned NFTs immediately. Tier is drawn from on-chain
// pseudo-randomness, weighted by remaining supply, and capped at 5 per wallet.
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(mint_index: u64)]
pub struct DevMint<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_CONFIG],
        bump = config.bump,
        constraint = !config.paused @ BitcoinError::Paused
    )]
    pub config: Account<'info, Config>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + UserState::INIT_SPACE,
        seeds = [SEED_USER, user.key().as_ref()],
        bump
    )]
    pub user_state: Account<'info, UserState>,

    /// Temporary miner record, seeded by [SEED_MINER, user, mint index]. Distinct
    /// from real (Metaplex) miners which are seeded by their nft mint. The handler
    /// requires `mint_index == user_state.total_minted` so indices stay sequential.
    #[account(
        init,
        payer = user,
        space = 8 + MinerState::INIT_SPACE,
        seeds = [SEED_MINER, user.key().as_ref(), &mint_index.to_le_bytes()],
        bump
    )]
    pub miner_state: Account<'info, MinerState>,

    pub system_program: Program<'info, System>,
}

pub fn dev_mint(ctx: Context<DevMint>, mint_index: u64) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let user_state = &mut ctx.accounts.user_state;

    if user_state.owner == Pubkey::default() {
        user_state.owner = ctx.accounts.user.key();
        user_state.bump = ctx.bumps.user_state;
    }

    // Indices must be sequential so the PDA matches total_minted.
    require!(
        mint_index == user_state.total_minted as u64,
        BitcoinError::InvalidParam
    );
    require!(
        user_state.total_minted < MAX_MINT_PER_WALLET,
        BitcoinError::MintLimitReached
    );
    require!(
        config.minted_total < TOTAL_NFT_SUPPLY,
        BitcoinError::SupplyExhausted
    );

    // On-chain pseudo-randomness. Not cryptographically secure (fine for the
    // temporary mint); uniqueness across a multi-mint tx comes from the nonce.
    let clock = Clock::get()?;
    let nonce = user_state.total_minted as u64;
    let h = keccak::hashv(&[
        ctx.accounts.user.key().as_ref(),
        &clock.slot.to_le_bytes(),
        &clock.unix_timestamp.to_le_bytes(),
        &nonce.to_le_bytes(),
    ]);
    let mut rb = [0u8; 8];
    rb.copy_from_slice(&h.0[..8]);
    let rand = u64::from_le_bytes(rb);

    let tier = weighted_tier(rand, &config.tier_remaining).ok_or(BitcoinError::SupplyExhausted)?;
    let ti = tier as usize;
    config.tier_remaining[ti] = config.tier_remaining[ti]
        .checked_sub(1)
        .ok_or(BitcoinError::SupplyExhausted)?;
    config.minted_total = config
        .minted_total
        .checked_add(1)
        .ok_or(BitcoinError::MathOverflow)?;
    let hashrate = TIER_HASHRATE[ti];

    let miner_key = ctx.accounts.miner_state.key();
    let miner = &mut ctx.accounts.miner_state;
    miner.owner = ctx.accounts.user.key();
    miner.nft_mint = miner_key; // placeholder until a real mint is attached
    miner.tier = tier;
    miner.hashrate = hashrate;
    miner.active = false;
    miner.tree_slot = 0;
    miner.team = Pubkey::default();
    miner.team_reward_debt = 0;
    miner.pending = 0;
    miner.blocks_won = 0;
    miner.total_earned = 0;
    miner.created_at = clock.unix_timestamp;
    miner.lock_until = 0;
    miner.bump = ctx.bumps.miner_state;

    user_state.total_minted = user_state
        .total_minted
        .checked_add(1)
        .ok_or(BitcoinError::MathOverflow)?;
    user_state.mint_nonce = user_state.mint_nonce.saturating_add(1);

    emit!(MintRevealed {
        user: miner.owner,
        nft_mint: miner.nft_mint,
        tier,
        hashrate,
        minted_total: config.minted_total,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct RequestMint<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_CONFIG],
        bump = config.bump,
        constraint = !config.paused @ BitcoinError::Paused
    )]
    pub config: Account<'info, Config>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + UserState::INIT_SPACE,
        seeds = [SEED_USER, user.key().as_ref()],
        bump
    )]
    pub user_state: Account<'info, UserState>,

    #[account(
        mut,
        address = config.token_mint @ BitcoinError::MintMismatch
    )]
    pub token_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_token.mint == config.token_mint @ BitcoinError::MintMismatch,
        constraint = user_token.owner == user.key() @ BitcoinError::OwnerMismatch
    )]
    pub user_token: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = user,
        space = 8 + PendingMint::INIT_SPACE,
        seeds = [SEED_PENDING_MINT, user.key().as_ref(), &user_state.mint_nonce.to_le_bytes()],
        bump
    )]
    pub pending_mint: Account<'info, PendingMint>,

    /// CHECK: Switchboard On-Demand randomness account (validated on settle).
    pub randomness: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn request_mint(ctx: Context<RequestMint>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let user_state = &mut ctx.accounts.user_state;

    // Initialize user_state on first use.
    if user_state.owner == Pubkey::default() {
        user_state.owner = ctx.accounts.user.key();
        user_state.active_count = 0;
        user_state.mint_nonce = 0;
        user_state.total_minted = 0;
        user_state.bump = ctx.bumps.user_state;
    }

    // Must still have mintable supply.
    require!(
        config.minted_total < TOTAL_NFT_SUPPLY,
        BitcoinError::SupplyExhausted
    );

    // Burn the mint price.
    let price = config.mint_price;
    require!(price > 0, BitcoinError::InvalidParam);
    burn(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Burn {
                mint: ctx.accounts.token_mint.to_account_info(),
                from: ctx.accounts.user_token.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        ),
        price,
    )?;
    config.total_burned = config
        .total_burned
        .checked_add(price)
        .ok_or(BitcoinError::MathOverflow)?;

    let nonce = user_state.mint_nonce;
    let pending = &mut ctx.accounts.pending_mint;
    pending.user = ctx.accounts.user.key();
    pending.nonce = nonce;
    pending.randomness = ctx.accounts.randomness.key();
    pending.commit_slot = Clock::get()?.slot;
    pending.settled = false;
    pending.bump = ctx.bumps.pending_mint;

    user_state.mint_nonce = nonce.checked_add(1).ok_or(BitcoinError::MathOverflow)?;

    emit!(MintRequested {
        user: pending.user,
        nonce,
        randomness: pending.randomness,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct SettleMint<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        close = recipient,
        seeds = [SEED_PENDING_MINT, pending_mint.user.as_ref(), &pending_mint.nonce.to_le_bytes()],
        bump = pending_mint.bump,
        constraint = !pending_mint.settled @ BitcoinError::MintAlreadySettled
    )]
    pub pending_mint: Account<'info, PendingMint>,

    /// CHECK: The NFT recipient (pending_mint.user); also the rent refund target.
    #[account(mut, address = pending_mint.user @ BitcoinError::Unauthorized)]
    pub recipient: UncheckedAccount<'info>,

    /// CHECK: Validated against pending_mint.randomness inside the handler.
    pub randomness: UncheckedAccount<'info>,

    /// CHECK: PDA mint/update authority.
    #[account(seeds = [SEED_MINT_AUTH], bump = config.mint_auth_bump)]
    pub mint_authority: UncheckedAccount<'info>,

    #[account(
        init,
        payer = payer,
        mint::decimals = 0,
        mint::authority = mint_authority,
        mint::freeze_authority = mint_authority
    )]
    pub nft_mint: Account<'info, Mint>,

    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = nft_mint,
        associated_token::authority = recipient
    )]
    pub nft_token: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = payer,
        space = 8 + MinerState::INIT_SPACE,
        seeds = [SEED_MINER, nft_mint.key().as_ref()],
        bump
    )]
    pub miner_state: Account<'info, MinerState>,

    /// CHECK: Created by Token Metadata CPI.
    #[account(mut)]
    pub metadata: UncheckedAccount<'info>,
    /// CHECK: Created by Token Metadata CPI.
    #[account(mut)]
    pub master_edition: UncheckedAccount<'info>,

    #[account(address = config.collection_mint @ BitcoinError::MintMismatch)]
    pub collection_mint: Account<'info, Mint>,
    /// CHECK: Collection metadata, validated by Token Metadata.
    #[account(mut)]
    pub collection_metadata: UncheckedAccount<'info>,
    /// CHECK: Collection master edition, validated by Token Metadata.
    pub collection_master_edition: UncheckedAccount<'info>,

    pub token_metadata_program: Program<'info, Metadata>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn settle_mint(ctx: Context<SettleMint>, name: String, uri: String) -> Result<()> {
    // Resolve randomness.
    let value = read_randomness(
        &ctx.accounts.randomness.to_account_info(),
        &ctx.accounts.pending_mint.randomness,
    )?;
    let rand = expand_u64(&value, 0);

    // Pick a tier weighted by remaining supply, then decrement.
    let config = &mut ctx.accounts.config;
    let tier = weighted_tier(rand, &config.tier_remaining).ok_or(BitcoinError::SupplyExhausted)?;
    let ti = tier as usize;
    config.tier_remaining[ti] = config.tier_remaining[ti]
        .checked_sub(1)
        .ok_or(BitcoinError::SupplyExhausted)?;
    config.minted_total = config
        .minted_total
        .checked_add(1)
        .ok_or(BitcoinError::MathOverflow)?;
    require!(
        config.minted_total <= TOTAL_NFT_SUPPLY,
        BitcoinError::SupplyExhausted
    );
    let hashrate = TIER_HASHRATE[ti];

    let mint_seeds: &[&[u8]] = &[SEED_MINT_AUTH, &[config.mint_auth_bump]];
    let signer = &[mint_seeds];

    // Mint the single NFT token.
    mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.nft_mint.to_account_info(),
                to: ctx.accounts.nft_token.to_account_info(),
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
                mint: ctx.accounts.nft_mint.to_account_info(),
                mint_authority: ctx.accounts.mint_authority.to_account_info(),
                update_authority: ctx.accounts.mint_authority.to_account_info(),
                payer: ctx.accounts.payer.to_account_info(),
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
                edition: ctx.accounts.master_edition.to_account_info(),
                mint: ctx.accounts.nft_mint.to_account_info(),
                update_authority: ctx.accounts.mint_authority.to_account_info(),
                mint_authority: ctx.accounts.mint_authority.to_account_info(),
                payer: ctx.accounts.payer.to_account_info(),
                metadata: ctx.accounts.metadata.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
                rent: ctx.accounts.rent.to_account_info(),
            },
            signer,
        ),
        Some(0),
    )?;

    // Verify the item into the sized collection.
    verify_sized_collection_item(
        CpiContext::new_with_signer(
            ctx.accounts.token_metadata_program.to_account_info(),
            VerifySizedCollectionItem {
                payer: ctx.accounts.payer.to_account_info(),
                metadata: ctx.accounts.metadata.to_account_info(),
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

    // Initialize miner state.
    let miner = &mut ctx.accounts.miner_state;
    miner.owner = ctx.accounts.recipient.key();
    miner.nft_mint = ctx.accounts.nft_mint.key();
    miner.tier = tier;
    miner.hashrate = hashrate;
    miner.active = false;
    miner.tree_slot = 0;
    miner.team = Pubkey::default();
    miner.team_reward_debt = 0;
    miner.pending = 0;
    miner.blocks_won = 0;
    miner.total_earned = 0;
    miner.created_at = Clock::get()?.unix_timestamp;
    miner.lock_until = 0;
    miner.bump = ctx.bumps.miner_state;

    emit!(MintRevealed {
        user: ctx.accounts.recipient.key(),
        nft_mint: miner.nft_mint,
        tier,
        hashrate,
        minted_total: config.minted_total,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// mint_nft: real, single-transaction Metaplex NFT mint (no VRF, no collection).
// The tier is drawn from on-chain pseudo-randomness, weighted by remaining
// supply. The NFT carries a `uri` that the app serves with live attributes
// (Rarity, Hashpower, Status, description). Capped at 5 per wallet.
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct MintNft<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_CONFIG],
        bump = config.bump,
        constraint = !config.paused @ BitcoinError::Paused,
        constraint = config.game_enabled @ BitcoinError::GameDisabled
    )]
    pub config: Account<'info, Config>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + UserState::INIT_SPACE,
        seeds = [SEED_USER, user.key().as_ref()],
        bump
    )]
    pub user_state: Account<'info, UserState>,

    #[account(mut, address = config.token_mint @ BitcoinError::MintMismatch)]
    pub token_mint: Account<'info, Mint>,

    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = token_mint,
        associated_token::authority = user
    )]
    pub user_token: Account<'info, TokenAccount>,

    /// CHECK: PDA mint/update authority.
    #[account(seeds = [SEED_MINT_AUTH], bump = config.mint_auth_bump)]
    pub mint_authority: UncheckedAccount<'info>,

    #[account(
        init,
        payer = user,
        mint::decimals = 0,
        mint::authority = mint_authority,
        mint::freeze_authority = mint_authority
    )]
    pub nft_mint: Account<'info, Mint>,

    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = nft_mint,
        associated_token::authority = user
    )]
    pub nft_token: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = user,
        space = 8 + MinerState::INIT_SPACE,
        seeds = [SEED_MINER, nft_mint.key().as_ref()],
        bump
    )]
    pub miner_state: Account<'info, MinerState>,

    /// CHECK: Created by Token Metadata CPI.
    #[account(mut)]
    pub metadata: UncheckedAccount<'info>,
    /// CHECK: Created by Token Metadata CPI.
    #[account(mut)]
    pub master_edition: UncheckedAccount<'info>,

    #[account(mut, address = config.miner_tree @ BitcoinError::InvalidParam)]
    pub miner_tree: AccountLoader<'info, MinerTree>,

    /// Optional team account, required only when the minter is in a team.
    #[account(mut)]
    pub team: Option<Account<'info, Team>>,

    /// CHECK: blacklist marker PDA [SEED_BLACKLIST, user]; validated in handler.
    pub blacklist: UncheckedAccount<'info>,

    pub token_metadata_program: Program<'info, Metadata>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn mint_nft(ctx: Context<MintNft>, name: String, uri: String) -> Result<()> {
    require_not_blacklisted(
        &ctx.accounts.blacklist.to_account_info(),
        ctx.program_id,
        &ctx.accounts.user.key(),
    )?;

    if ctx.accounts.user_state.owner == Pubkey::default() {
        ctx.accounts.user_state.owner = ctx.accounts.user.key();
        ctx.accounts.user_state.bump = ctx.bumps.user_state;
    }

    require!(
        ctx.accounts.user_state.total_minted < MAX_MINT_PER_WALLET,
        BitcoinError::MintLimitReached
    );
    require!(
        ctx.accounts.config.minted_total < TOTAL_NFT_SUPPLY,
        BitcoinError::SupplyExhausted
    );

    // Burn the mint price (0 = free; admin-configurable).
    let price = ctx.accounts.config.mint_price;
    if price > 0 {
        burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.token_mint.to_account_info(),
                    from: ctx.accounts.user_token.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            price,
        )?;
        ctx.accounts.config.total_burned = ctx
            .accounts
            .config
            .total_burned
            .checked_add(price)
            .ok_or(BitcoinError::MathOverflow)?;
    }

    // On-chain pseudo-randomness, weighted by remaining supply (accurate odds).
    let clock = Clock::get()?;
    let nonce = ctx.accounts.user_state.total_minted as u64;
    let h = keccak::hashv(&[
        ctx.accounts.nft_mint.key().as_ref(),
        ctx.accounts.user.key().as_ref(),
        &clock.slot.to_le_bytes(),
        &clock.unix_timestamp.to_le_bytes(),
        &nonce.to_le_bytes(),
    ]);
    let mut rb = [0u8; 8];
    rb.copy_from_slice(&h.0[..8]);
    let rand = u64::from_le_bytes(rb);

    let tier =
        weighted_tier(rand, &ctx.accounts.config.tier_remaining).ok_or(BitcoinError::SupplyExhausted)?;
    let ti = tier as usize;
    ctx.accounts.config.tier_remaining[ti] = ctx.accounts.config.tier_remaining[ti]
        .checked_sub(1)
        .ok_or(BitcoinError::SupplyExhausted)?;
    ctx.accounts.config.minted_total = ctx
        .accounts
        .config
        .minted_total
        .checked_add(1)
        .ok_or(BitcoinError::MathOverflow)?;

    let hashrate = ctx.accounts.config.tier_hashrate[ti];
    let max_active_hr = ctx.accounts.config.max_active_hr;
    let mint_auth_bump = ctx.accounts.config.mint_auth_bump;
    let nft_mint_key = ctx.accounts.nft_mint.key();

    let mint_seeds: &[&[u8]] = &[SEED_MINT_AUTH, &[mint_auth_bump]];
    let signer = &[mint_seeds];

    mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.nft_mint.to_account_info(),
                to: ctx.accounts.nft_token.to_account_info(),
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
                mint: ctx.accounts.nft_mint.to_account_info(),
                mint_authority: ctx.accounts.mint_authority.to_account_info(),
                update_authority: ctx.accounts.mint_authority.to_account_info(),
                payer: ctx.accounts.user.to_account_info(),
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
            collection: None,
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
                edition: ctx.accounts.master_edition.to_account_info(),
                mint: ctx.accounts.nft_mint.to_account_info(),
                update_authority: ctx.accounts.mint_authority.to_account_info(),
                mint_authority: ctx.accounts.mint_authority.to_account_info(),
                payer: ctx.accounts.user.to_account_info(),
                metadata: ctx.accounts.metadata.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
                rent: ctx.accounts.rent.to_account_info(),
            },
            signer,
        ),
        Some(0),
    )?;

    // Base miner record (inactive by default).
    {
        let miner = &mut ctx.accounts.miner_state;
        miner.owner = ctx.accounts.user.key();
        miner.nft_mint = nft_mint_key;
        miner.tier = tier;
        miner.hashrate = hashrate;
        miner.active = false;
        miner.tree_slot = 0;
        miner.team = Pubkey::default();
        miner.team_reward_debt = 0;
        miner.pending = 0;
        miner.blocks_won = 0;
        miner.total_earned = 0;
        miner.created_at = clock.unix_timestamp;
        miner.lock_until = 0;
        miner.bump = ctx.bumps.miner_state;
    }

    // Active by default, as long as it fits under the wallet's hashrate cap.
    let team_pubkey = ctx.accounts.user_state.team;
    let can_activate =
        ctx.accounts.user_state.active_hashrate.saturating_add(hashrate) <= max_active_hr;
    if can_activate {
        let slot = {
            let mut tree = ctx.accounts.miner_tree.load_mut()?;
            tree.insert(nft_mint_key, hashrate)?
        };
        {
            let miner = &mut ctx.accounts.miner_state;
            miner.active = true;
            miner.tree_slot = slot;
            miner.team = team_pubkey;
        }
        if team_pubkey != Pubkey::default() {
            let acc = {
                let team = ctx.accounts.team.as_mut().ok_or(BitcoinError::NotInTeam)?;
                require_keys_eq!(team.key(), team_pubkey, BitcoinError::NotInTeam);
                team.total_active_hashrate = team
                    .total_active_hashrate
                    .checked_add(hashrate)
                    .ok_or(BitcoinError::MathOverflow)?;
                team.acc_reward_per_hashrate
            };
            ctx.accounts.miner_state.team_reward_debt = acc;
        }
        let us = &mut ctx.accounts.user_state;
        us.active_hashrate = us.active_hashrate.saturating_add(hashrate);
        us.active_count = us.active_count.saturating_add(1);
    }

    ctx.accounts.user_state.total_minted = ctx
        .accounts
        .user_state
        .total_minted
        .checked_add(1)
        .ok_or(BitcoinError::MathOverflow)?;

    emit!(MintRevealed {
        user: ctx.accounts.user.key(),
        nft_mint: nft_mint_key,
        tier,
        hashrate,
        minted_total: ctx.accounts.config.minted_total,
    });
    Ok(())
}
