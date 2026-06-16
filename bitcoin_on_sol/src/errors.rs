use anchor_lang::prelude::*;

#[error_code]
pub enum BitcoinError {
    #[msg("Program is paused")]
    Paused,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Invalid parameter")]
    InvalidParam,

    // Mint
    #[msg("All 2,100 NFTs have already been minted")]
    SupplyExhausted,
    #[msg("Pending mint already settled")]
    MintAlreadySettled,
    #[msg("Randomness is not yet resolved")]
    RandomnessNotResolved,
    #[msg("Randomness account mismatch")]
    RandomnessAccountMismatch,
    #[msg("Randomness is stale or expired")]
    RandomnessStale,

    // Lifecycle
    #[msg("Miner is already active")]
    AlreadyActive,
    #[msg("Miner is not active")]
    NotActive,
    #[msg("Miner must be inactive for this action")]
    MustBeInactive,
    #[msg("User has reached the maximum number of active miners")]
    TooManyActiveMiners,
    #[msg("Active-miner lottery tree is full")]
    TreeFull,
    #[msg("Invalid lottery tree slot")]
    InvalidSlot,
    #[msg("Total network hashrate is zero; no eligible miners")]
    NoActiveMiners,
    #[msg("Miner is locked for 12 hours after activation")]
    MinerLocked,

    // Teams
    #[msg("Miner is already in a team")]
    AlreadyInTeam,
    #[msg("Miner is not in this team")]
    NotInTeam,
    #[msg("Team name too long")]
    NameTooLong,
    #[msg("Team is not empty")]
    TeamNotEmpty,
    #[msg("Miner must leave its team first")]
    MustLeaveTeam,
    #[msg("An invite from the team is required to join")]
    InviteRequired,
    #[msg("Team creation is currently disabled")]
    TeamsDisabled,
    #[msg("Team name must be 1-32 letters or digits only")]
    InvalidTeamName,
    #[msg("Team is full")]
    TeamFull,
    #[msg("Invite id must be a 10-digit number")]
    InvalidInviteId,

    // Sacrifice / upgrade
    #[msg("Both NFTs must be the same tier")]
    TierMismatch,
    #[msg("Cannot sacrifice into Grail; Grails are mint-only")]
    GrailNotForgeable,
    #[msg("Cannot use the same NFT twice")]
    DuplicateNft,

    // Crank / blocks
    #[msg("Block interval has not elapsed yet")]
    BlockTooSoon,
    #[msg("Block round already settled")]
    RoundAlreadySettled,
    #[msg("Block round not yet committed")]
    RoundNotCommitted,
    #[msg("Invalid block kind")]
    InvalidBlockKind,

    // Claims
    #[msg("Not a recorded winner for this round")]
    NotAWinner,
    #[msg("This win was already collected")]
    AlreadyCollected,
    #[msg("Winner account mismatch")]
    WinnerMismatch,
    #[msg("Nothing to claim")]
    NothingToClaim,
    #[msg("Reward pool is insufficient")]
    PoolInsufficient,

    // Misc account validation
    #[msg("Token account owner mismatch")]
    OwnerMismatch,
    #[msg("Mint mismatch")]
    MintMismatch,
    #[msg("Not the owner of this NFT")]
    NotNftOwner,
}
