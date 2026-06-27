use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::Metadata;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::{
    CustodyAllocation, ErrorCode, GraiState, JuniorVault, SeniorVault,
};
use crate::price_feed;

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = GraiState::space(0),
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        init,
        payer = authority,
        mint::decimals = GraiState::DECIMALS,
        mint::authority = grai_state,
    )]
    pub grai_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub token_metadata_program: Program<'info, Metadata>,

    /// CHECK: Metaplex metadata PDA for `grai_mint`.
    #[account(
        mut,
        seeds = [b"metadata", token_metadata_program.key().as_ref(), grai_mint.key().as_ref()],
        bump,
        seeds::program = token_metadata_program.key(),
    )]
    pub metadata: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct SetTreasury<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
#[instruction(price_feed_key: Pubkey)]
pub struct SetPriceFeed<'info> {
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub senior_vault: Account<'info, SeniorVault>,

    /// CHECK: Chainlink, Pyth, or program-owned custom price feed for `asset_mint`.
    #[account(
        constraint = price_feed.key() == price_feed_key @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[instruction(paused_minting: bool)]
pub struct SetPausedMinting<'info> {
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub senior_vault: Account<'info, SeniorVault>,
}

#[derive(Accounts)]
#[instruction(mint_split: u16)]
pub struct SetMintSplit<'info> {
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub senior_vault: Account<'info, SeniorVault>,
}

#[derive(Accounts)]
#[instruction(yield_split: u16)]
pub struct SetYieldSplit<'info> {
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub senior_vault: Account<'info, SeniorVault>,
}

#[derive(Accounts)]
pub struct AddAsset<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump,
        has_one = authority @ ErrorCode::Unauthorized,
        realloc = GraiState::space(grai_state.asset_mints.len() + 1),
        realloc::payer = authority,
        realloc::zero = false,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        init,
        payer = authority,
        space = 8 + JuniorVault::LEN,
        seeds = [JuniorVault::SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub junior_vault: Account<'info, JuniorVault>,

    #[account(
        init,
        payer = authority,
        space = 8 + SeniorVault::LEN,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub senior_vault: Account<'info, SeniorVault>,

    #[account(
        init_if_needed,
        payer = authority,
        token::mint = asset_mint,
        token::authority = grai_state,
        seeds = [JuniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub junior_vault_ata: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = authority,
        token::mint = asset_mint,
        token::authority = grai_state,
        seeds = [SeniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub senior_vault_ata: Account<'info, TokenAccount>,

    /// CHECK: Chainlink, Pyth, or program-owned custom price feed for `asset_mint`.
    #[account(
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct RemoveAsset<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump,
        has_one = authority @ ErrorCode::Unauthorized,
        realloc = GraiState::space(grai_state.asset_mints.len().saturating_sub(1)),
        realloc::payer = authority,
        realloc::zero = false,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        close = authority,
        seeds = [JuniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = junior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub junior_vault: Account<'info, JuniorVault>,
    
    #[account(
        mut,
        close = authority,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        constraint = senior_vault.paused_minting @ ErrorCode::AssetMintingEnabled,
    )]
    pub senior_vault: Account<'info, SeniorVault>,

    #[account(
        mut,
        seeds = [SeniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub senior_vault_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [JuniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = junior_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub junior_vault_ata: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = asset_mint,
        associated_token::authority = authority,
    )]
    pub authority_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct MintToken<'info> {
    #[account(mut)]
    pub minter: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    /// CHECK: Chainlink, Pyth, or program-owned custom price feed for `asset_mint`.
    #[account(
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        constraint = !senior_vault.paused_minting @ ErrorCode::AssetMintingPaused,
        has_one = price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub senior_vault: Box<Account<'info, SeniorVault>>,

    #[account(
        mut,
        seeds = [SeniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub senior_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [JuniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = junior_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub junior_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = minter_ata.mint == asset_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = minter_ata.owner == minter.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub minter_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = minter,
        associated_token::mint = grai_mint,
        associated_token::authority = minter,
    )]
    pub minter_grai_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct MintSol<'info> {
    #[account(mut)]
    pub minter: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        address = anchor_spl::token::spl_token::native_mint::ID @ ErrorCode::InvalidMint,
    )]
    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    /// CHECK: Chainlink, Pyth, or program-owned custom price feed for `asset_mint`.
    #[account(
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        constraint = !senior_vault.paused_minting @ ErrorCode::AssetMintingPaused,
        has_one = price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub senior_vault: Box<Account<'info, SeniorVault>>,

    #[account(
        mut,
        seeds = [SeniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub senior_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [JuniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = junior_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub junior_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = minter,
        associated_token::mint = asset_mint,
        associated_token::authority = minter,
    )]
    pub minter_wsol_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = minter,
        associated_token::mint = grai_mint,
        associated_token::authority = minter,
    )]
    pub minter_grai_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BurnGrai<'info> {
    pub burner: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        constraint = burner_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = burner_grai_ata.owner == burner.key() @ ErrorCode::InvalidDestination,
    )]
    pub burner_grai_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct Allocate<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        seeds = [JuniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = junior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub junior_vault: Account<'info, JuniorVault>,

    #[account(
        mut,
        seeds = [JuniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = junior_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        constraint = amount > 0 @ ErrorCode::InvalidAmount,
        constraint = junior_vault_ata.amount >= amount @ ErrorCode::InsufficientActiveCapital,
    )]
    pub junior_vault_ata: Account<'info, TokenAccount>,

    /// CHECK: custody wallet; ATA owner and custody_allocation seed.
    #[account(
        constraint = custody_wallet.key() != Pubkey::default() @ ErrorCode::InvalidCustody,
    )]
    pub custody_wallet: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = asset_mint,
        associated_token::authority = custody_wallet,
    )]
    pub custody_ata: Account<'info, TokenAccount>,
    
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + CustodyAllocation::LEN,
        seeds = [
            CustodyAllocation::SEED,
            custody_wallet.key().as_ref(),
            asset_mint.key().as_ref(),
        ],
        bump,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(yield_amount: u64)]
pub struct Distribute<'info> {
    pub custody_wallet: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    pub asset_mint: Account<'info, Mint>,

    /// CHECK: Chainlink, Pyth, or program-owned custom price feed for `asset_mint`.
    #[account(
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        has_one = price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub senior_vault: Account<'info, SeniorVault>,

    #[account(
        mut,
        seeds = [
            CustodyAllocation::SEED,
            custody_wallet.key().as_ref(),
            asset_mint.key().as_ref(),
        ],
        bump,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    #[account(
        mut,
        constraint = custody_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = custody_ata.owner == custody_wallet.key() @ ErrorCode::InvalidCustody,
    )]
    pub custody_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [SeniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = senior_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub senior_vault_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = treasury_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = treasury_ata.owner == grai_state.treasury_wallet @ ErrorCode::InvalidDestination,
    )]
    pub treasury_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct GetNav<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct GetAssets<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct GetVaults<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Account<'info, GraiState>,
}
