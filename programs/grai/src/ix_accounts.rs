use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::metadata::Metadata;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::{
    CustodyAllocation, ErrorCode, AssetVaultState, MintConfig, GraiState,
};

#[derive(Accounts)]
pub struct InitializeToken<'info> {
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
        space = 8 + MintConfig::LEN,
        seeds = [MintConfig::SEED],
        bump,
    )]
    pub mint_config: Account<'info, MintConfig>,

    #[account(
        init,
        payer = authority,
        mint::decimals = MintConfig::DECIMALS,
        mint::authority = mint_config,
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
        bump = grai_state.bump,
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
        bump = grai_state.bump,
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
        bump = grai_state.bump,
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
        token::mint = asset_mint,
        token::authority = grai_state,
        seeds = [AssetVaultState::GRAI_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub grai_vault: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = authority,
        token::mint = asset_mint,
        token::authority = grai_state,
        seeds = [AssetVaultState::ASSET_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub asset_vault: Account<'info, TokenAccount>,

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
        bump = grai_state.bump,
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
        seeds = [AssetVaultState::GRAI_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault.key() == asset_vault_state.grai_vault @ ErrorCode::InvalidVault,
        constraint = grai_vault.amount == 0 @ ErrorCode::VaultNotEmpty,
    )]
    pub grai_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [AssetVaultState::ASSET_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault.key() == asset_vault_state.asset_vault @ ErrorCode::InvalidVault,
        constraint = asset_vault.amount == 0 @ ErrorCode::VaultNotEmpty,
    )]
    pub asset_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(minting: bool)]
pub struct SetMinting<'info> {
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
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
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetVaultState::GRAI_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = grai_vault.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub grai_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetVaultState::ASSET_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault.mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: Chainlink v2 feed or program-owned custom price feed.
    pub price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [AssetVaultState::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
        has_one = grai_vault @ ErrorCode::InvalidVault,
        constraint = asset_vault.key() == asset_vault_state.asset_vault @ ErrorCode::InvalidVault,
        has_one = price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = asset_vault_state.minting @ ErrorCode::AssetMintingPaused,
    )]
    pub asset_vault_state: Box<Account<'info, AssetVaultState>>,

    #[account(
        mut,
        constraint = minter_token_account.mint == asset_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = minter_token_account.owner == minter.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub minter_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        seeds = [MintConfig::SEED],
        bump = mint_config.bump,
    )]
    pub mint_config: Box<Account<'info, MintConfig>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(mint_config.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = minter_grai_account.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = minter_grai_account.owner == minter.key() @ ErrorCode::InvalidDestination,
    )]
    pub minter_grai_account: Box<Account<'info, TokenAccount>>,

    pub clock: Sysvar<'info, Clock>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct BurnGrai<'info> {
    pub redeemer: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        constraint = redeemer_grai_account.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = redeemer_grai_account.owner == redeemer.key() @ ErrorCode::InvalidDestination,
    )]
    pub redeemer_grai_account: Account<'info, TokenAccount>,

    #[account(
        seeds = [MintConfig::SEED],
        bump = mint_config.bump,
    )]
    pub mint_config: Account<'info, MintConfig>,

    #[account(mut)]
    pub grai_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(amount: u64, custody_wallet: Pubkey)]
pub struct Allocate<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
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
        init_if_needed,
        payer = authority,
        space = 8 + CustodyAllocation::LEN,
        seeds = [
            CustodyAllocation::SEED,
            custody_wallet.as_ref(),
            asset_mint.key().as_ref(),
        ],
        bump,
        constraint = custody_wallet != Pubkey::default() @ ErrorCode::InvalidCustody,
        constraint = custody_allocation.asset_mint == Pubkey::default()
            || custody_allocation.custody_wallet == custody_wallet @ ErrorCode::InvalidCustody,
        constraint = custody_allocation.asset_mint == Pubkey::default()
            || custody_allocation.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    #[account(
        mut,
        constraint = grai_vault.key() == asset_vault_state.grai_vault @ ErrorCode::InvalidVault,
    )]
    pub grai_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = custody_token_account.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = custody_token_account.owner == custody_wallet @ ErrorCode::InvalidCustody,
    )]
    pub custody_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount: u64, custody_wallet: Pubkey)]
pub struct Deallocate<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub custody_wallet: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
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
        seeds = [
            CustodyAllocation::SEED,
            custody_wallet.key().as_ref(),
            asset_mint.key().as_ref(),
        ],
        bump,
        constraint = custody_allocation.custody_wallet == custody_wallet.key() @ ErrorCode::InvalidCustody,
        constraint = custody_allocation.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    #[account(
        mut,
        constraint = grai_vault.key() == asset_vault_state.grai_vault @ ErrorCode::InvalidVault,
    )]
    pub grai_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = custody_token_account.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = custody_token_account.owner == custody_wallet.key() @ ErrorCode::InvalidCustody,
    )]
    pub custody_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(yield_amount: u64, custody_wallet: Pubkey)]
pub struct Distribute<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub custody_wallet: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
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
        seeds = [
            CustodyAllocation::SEED,
            custody_wallet.key().as_ref(),
            asset_mint.key().as_ref(),
        ],
        bump,
        constraint = custody_allocation.custody_wallet == custody_wallet.key() @ ErrorCode::InvalidCustody,
        constraint = custody_allocation.asset_mint == asset_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    #[account(
        mut,
        constraint = custody_token_account.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = custody_token_account.owner == custody_wallet.key() @ ErrorCode::InvalidCustody,
    )]
    pub custody_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = grai_vault.key() == asset_vault_state.grai_vault @ ErrorCode::InvalidVault,
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
