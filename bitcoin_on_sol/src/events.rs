use anchor_lang::prelude::*;

#[event]
pub struct MintRequested {
    pub user: Pubkey,
    pub nonce: u64,
    pub randomness: Pubkey,
}

#[event]
pub struct MintRevealed {
    pub user: Pubkey,
    pub nft_mint: Pubkey,
    pub tier: u8,
    pub hashrate: u64,
    pub minted_total: u32,
}

#[event]
pub struct MinerActivated {
    pub nft_mint: Pubkey,
    pub owner: Pubkey,
    pub hashrate: u64,
    pub slot: u32,
}

#[event]
pub struct MinerDeactivated {
    pub nft_mint: Pubkey,
    pub owner: Pubkey,
}

#[event]
pub struct BlockCommitted {
    pub kind: u8,
    pub index: u64,
    pub reward_each: u64,
    pub timestamp: i64,
}

#[event]
pub struct BlockWon {
    pub kind: u8,
    pub index: u64,
    pub nft_mint: Pubkey,
    pub reward: u64,
    pub timestamp: i64,
}

#[event]
pub struct WinCollected {
    pub kind: u8,
    pub index: u64,
    pub nft_mint: Pubkey,
    pub reward: u64,
    pub to_team: bool,
}

#[event]
pub struct Sacrificed {
    pub owner: Pubkey,
    pub from_tier: u8,
    pub to_tier: u8,
    pub burned_a: Pubkey,
    pub burned_b: Pubkey,
    pub new_mint: Pubkey,
}

#[event]
pub struct RewardsClaimed {
    pub owner: Pubkey,
    pub amount: u64,
}

#[event]
pub struct TeamCreated {
    pub team: Pubkey,
    pub authority: Pubkey,
    pub fee_paid: u64,
}

#[event]
pub struct InviteCreated {
    pub team: Pubkey,
    pub invitee: Pubkey,
    pub invite_id: u64,
}

#[event]
pub struct TeamMembershipChanged {
    pub team: Pubkey,
    /// The wallet whose membership changed.
    pub member: Pubkey,
    /// true = joined, false = left/kicked.
    pub joined: bool,
    /// true when removed by an admin kick.
    pub by_admin: bool,
}

#[event]
pub struct TeamDisbanded {
    pub team: Pubkey,
    pub authority: Pubkey,
}
