use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::Metadata;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::{
    CustodyAllocation, ErrorCode, AssetVaultState, GraiVaultState, GraiState,
};

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + GraiState::LEN,
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
        seeds = [AssetVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    /// CHECK: Chainlink v2 feed or program-owned custom price feed.
    pub price_feed: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct AddAssetVault<'info> {
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
        init,
        payer = authority,
        space = 8 + AssetVaultState::LEN,
        seeds = [AssetVaultState::SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    #[account(
        init,
        payer = authority,
        space = 8 + GraiVaultState::LEN,
        seeds = [GraiVaultState::STATE_SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub grai_vault_state: Account<'info, GraiVaultState>,

    #[account(
        init,
        payer = authority,
        token::mint = asset_mint,
        token::authority = grai_state,
        seeds = [GraiVaultState::SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub grai_vault_ata: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = authority,
        token::mint = asset_mint,
        token::authority = grai_state,
        seeds = [b"asset_vault", asset_mint.key().as_ref()],
        bump,
    )]
    pub asset_vault_ata: Account<'info, TokenAccount>,

    /// CHECK: Chainlink v2 feed or program-owned custom price feed.
    pub price_feed: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct RemoveAssetVault<'info> {
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
        close = authority,
        seeds = [AssetVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        constraint = !asset_vault_state.minting @ ErrorCode::AssetMintingEnabled,
        constraint = asset_vault_state.active_amount == 0 @ ErrorCode::ActiveCapitalDeployed,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    #[account(
        mut,
        close = authority,
        seeds = [GraiVaultState::STATE_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        constraint = grai_vault_state.idle_amount == 0 @ ErrorCode::VaultNotEmpty,
    )]
    pub grai_vault_state: Account<'info, GraiVaultState>,

    #[account(
        mut,
        seeds = [GraiVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.amount == 0 @ ErrorCode::VaultNotEmpty,
    )]
    pub grai_vault_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"asset_vault", asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault_ata.amount == 0 @ ErrorCode::VaultNotEmpty,
    )]
    pub asset_vault_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(minting: bool)]
pub struct SetMinting<'info> {
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
        seeds = [AssetVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct MintGrai<'info> {
    pub minter: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    /// CHECK: Chainlink v2 feed or program-owned custom price feed.
    pub price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [GraiVaultState::STATE_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub grai_vault_state: Box<Account<'info, GraiVaultState>>,

    #[account(
        mut,
        seeds = [GraiVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [b"asset_vault", asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        has_one = price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = asset_vault_state.minting @ ErrorCode::AssetMintingPaused,
    )]
    pub asset_vault_state: Box<Account<'info, AssetVaultState>>,

    #[account(
        mut,
        constraint = minter_ata.mint == asset_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = minter_ata.owner == minter.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub minter_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = minter_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = minter_grai_ata.owner == minter.key() @ ErrorCode::InvalidDestination,
    )]
    pub minter_grai_ata: Box<Account<'info, TokenAccount>>,

    pub clock: Sysvar<'info, Clock>,
    pub token_program: Program<'info, Token>,
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
        seeds = [AssetVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    #[account(
        mut,
        seeds = [GraiVaultState::STATE_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        constraint = amount > 0 @ ErrorCode::InvalidAmount,
        constraint = grai_vault_state.idle_amount >= amount @ ErrorCode::InsufficientIdleLiquidity,
    )]
    pub grai_vault_state: Account<'info, GraiVaultState>,

    #[account(
        mut,
        seeds = [GraiVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        constraint = grai_vault_ata.amount >= amount @ ErrorCode::InsufficientIdleLiquidity,
    )]
    pub grai_vault_ata: Account<'info, TokenAccount>,

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

    #[account(
        mut,
        seeds = [AssetVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    #[account(
        mut,
        seeds = [GraiVaultState::STATE_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub grai_vault_state: Account<'info, GraiVaultState>,

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
        seeds = [GraiVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub grai_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = treasury_token_account.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = treasury_token_account.owner == grai_state.treasury_wallet @ ErrorCode::InvalidDestination,
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,

    /// CHECK: Chainlink v2 feed or program-owned custom price feed.
    pub price_feed: UncheckedAccount<'info>,

    pub clock: Sysvar<'info, Clock>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CalcInternalValue<'info> {
    pub clock: Sysvar<'info, Clock>,
}
