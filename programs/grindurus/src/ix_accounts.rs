use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::{
    AssetRegistry, CustodyAllocation, ErrorCode, AssetVaultState, MintConfig, GraiState,
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
        space = 8 + AssetRegistry::LEN,
        seeds = [AssetRegistry::SEED],
        bump,
    )]
    pub asset_registry: Account<'info, AssetRegistry>,

    #[account(
        init,
        payer = authority,
        mint::decimals = MintConfig::DECIMALS,
        mint::authority = mint_config,
    )]
    pub grai_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct SetTreasury<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [AssetRegistry::SEED],
        bump = asset_registry.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub asset_registry: Account<'info, AssetRegistry>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
#[instruction(mint_pubkey: Pubkey, asset_kind: u8)]
pub struct AddAssetVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub accepted_mint: Account<'info, Mint>,

    #[account(
        seeds = [AssetRegistry::SEED],
        bump = asset_registry.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub asset_registry: Account<'info, AssetRegistry>,

    #[account(
        init,
        payer = authority,
        space = 8 + AssetVaultState::LEN,
        seeds = [AssetVaultState::SEED, mint_pubkey.as_ref()],
        bump,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    #[account(
        init,
        payer = authority,
        token::mint = accepted_mint,
        token::authority = asset_registry,
        seeds = [AssetVaultState::GRAI_VAULT_SEED, mint_pubkey.as_ref()],
        bump,
    )]
    pub grai_vault: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = authority,
        token::mint = accepted_mint,
        token::authority = asset_registry,
        seeds = [AssetVaultState::ASSET_VAULT_SEED, mint_pubkey.as_ref()],
        bump,
    )]
    pub asset_vault: Account<'info, TokenAccount>,

    /// CHECK: validated when reading price during mint.
    pub chainlink_feed: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction(mint_pubkey: Pubkey)]
pub struct RemoveAssetVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub accepted_mint: Account<'info, Mint>,

    #[account(
        seeds = [AssetRegistry::SEED],
        bump = asset_registry.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub asset_registry: Account<'info, AssetRegistry>,

    #[account(
        mut,
        close = authority,
        seeds = [AssetVaultState::SEED, mint_pubkey.as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
        constraint = !asset_vault_state.minting_enabled @ ErrorCode::AssetMintingEnabled,
        constraint = asset_vault_state.active_amount == 0 @ ErrorCode::ActiveCapitalDeployed,
        constraint = asset_vault_state.asset_amount == 0 @ ErrorCode::VaultNotEmpty,
        constraint = asset_vault_state.yield_amount == 0 @ ErrorCode::YieldNotRealized,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    #[account(
        mut,
        seeds = [AssetVaultState::GRAI_VAULT_SEED, mint_pubkey.as_ref()],
        bump,
        constraint = grai_vault.key() == asset_vault_state.grai_vault @ ErrorCode::InvalidVault,
        constraint = grai_vault.amount == 0 @ ErrorCode::VaultNotEmpty,
    )]
    pub grai_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [AssetVaultState::ASSET_VAULT_SEED, mint_pubkey.as_ref()],
        bump,
        constraint = asset_vault.key() == asset_vault_state.asset_vault @ ErrorCode::InvalidVault,
        constraint = asset_vault.amount == 0 @ ErrorCode::VaultNotEmpty,
    )]
    pub asset_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(mint_pubkey: Pubkey, paused: bool)]
pub struct SetPauseAssetVault<'info> {
    pub authority: Signer<'info>,

    pub accepted_mint: Account<'info, Mint>,

    #[account(
        seeds = [AssetRegistry::SEED],
        bump = asset_registry.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub asset_registry: Account<'info, AssetRegistry>,

    #[account(
        mut,
        seeds = [AssetVaultState::SEED, mint_pubkey.as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,
}

#[derive(Accounts)]
#[instruction(amount: u64, mint_pubkey: Pubkey)]
pub struct MintGrai<'info> {
    pub minter: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub accepted_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetVaultState::GRAI_VAULT_SEED, mint_pubkey.as_ref()],
        bump,
        constraint = grai_vault.mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub grai_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetVaultState::ASSET_VAULT_SEED, mint_pubkey.as_ref()],
        bump,
        constraint = asset_vault.mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: parsed by Chainlink SDK v2.
    pub chainlink_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [AssetVaultState::SEED, mint_pubkey.as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
        has_one = grai_vault @ ErrorCode::InvalidVault,
        constraint = asset_vault.key() == asset_vault_state.asset_vault @ ErrorCode::InvalidVault,
        has_one = chainlink_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = asset_vault_state.minting_enabled @ ErrorCode::AssetMintingPaused,
    )]
    pub asset_vault_state: Box<Account<'info, AssetVaultState>>,

    #[account(
        mut,
        constraint = minter_token_account.mint == accepted_mint.key() @ ErrorCode::InvalidDepositSource,
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
        seeds = [AssetRegistry::SEED],
        bump = asset_registry.bump,
    )]
    pub asset_registry: Account<'info, AssetRegistry>,

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
#[instruction(amount: u64, mint_pubkey: Pubkey, custody_wallet: Pubkey)]
pub struct Allocate<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub accepted_mint: Account<'info, Mint>,

    #[account(
        seeds = [AssetRegistry::SEED],
        bump = asset_registry.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub asset_registry: Account<'info, AssetRegistry>,

    #[account(
        mut,
        seeds = [AssetVaultState::SEED, mint_pubkey.as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + CustodyAllocation::LEN,
        seeds = [
            CustodyAllocation::SEED,
            custody_wallet.as_ref(),
            mint_pubkey.as_ref(),
        ],
        bump,
        constraint = custody_wallet != Pubkey::default() @ ErrorCode::InvalidCustody,
        constraint = custody_allocation.asset_mint == Pubkey::default()
            || custody_allocation.custody_wallet == custody_wallet @ ErrorCode::InvalidCustody,
        constraint = custody_allocation.asset_mint == Pubkey::default()
            || custody_allocation.asset_mint == mint_pubkey @ ErrorCode::InvalidGraiVault,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    #[account(
        mut,
        constraint = grai_vault.key() == asset_vault_state.grai_vault @ ErrorCode::InvalidVault,
    )]
    pub grai_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = custody_token_account.mint == accepted_mint.key() @ ErrorCode::InvalidDestination,
        constraint = custody_token_account.owner == custody_wallet @ ErrorCode::InvalidCustody,
    )]
    pub custody_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount: u64, mint_pubkey: Pubkey, custody_wallet: Pubkey)]
pub struct Deallocate<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub custody_wallet: Signer<'info>,

    pub accepted_mint: Account<'info, Mint>,

    #[account(
        seeds = [AssetRegistry::SEED],
        bump = asset_registry.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub asset_registry: Account<'info, AssetRegistry>,

    #[account(
        mut,
        seeds = [AssetVaultState::SEED, mint_pubkey.as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    #[account(
        mut,
        seeds = [
            CustodyAllocation::SEED,
            custody_wallet.key().as_ref(),
            mint_pubkey.as_ref(),
        ],
        bump,
        constraint = custody_allocation.custody_wallet == custody_wallet.key() @ ErrorCode::InvalidCustody,
        constraint = custody_allocation.asset_mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    #[account(
        mut,
        constraint = grai_vault.key() == asset_vault_state.grai_vault @ ErrorCode::InvalidVault,
    )]
    pub grai_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = custody_token_account.mint == accepted_mint.key() @ ErrorCode::InvalidDestination,
        constraint = custody_token_account.owner == custody_wallet.key() @ ErrorCode::InvalidCustody,
    )]
    pub custody_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(yield_amount: u64, mint_pubkey: Pubkey, custody_wallet: Pubkey)]
pub struct Distribute<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub custody_wallet: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    pub accepted_mint: Account<'info, Mint>,

    #[account(
        seeds = [AssetRegistry::SEED],
        bump = asset_registry.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub asset_registry: Account<'info, AssetRegistry>,

    #[account(
        mut,
        seeds = [AssetVaultState::SEED, mint_pubkey.as_ref()],
        bump,
        constraint = asset_vault_state.asset_mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub asset_vault_state: Account<'info, AssetVaultState>,

    #[account(
        mut,
        seeds = [
            CustodyAllocation::SEED,
            custody_wallet.key().as_ref(),
            mint_pubkey.as_ref(),
        ],
        bump,
        constraint = custody_allocation.custody_wallet == custody_wallet.key() @ ErrorCode::InvalidCustody,
        constraint = custody_allocation.asset_mint == accepted_mint.key() @ ErrorCode::InvalidGraiVault,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    #[account(
        mut,
        constraint = custody_token_account.mint == accepted_mint.key() @ ErrorCode::InvalidDestination,
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
        constraint = treasury_token_account.mint == accepted_mint.key() @ ErrorCode::InvalidDestination,
        constraint = treasury_token_account.owner == grai_state.treasury_wallet @ ErrorCode::InvalidDestination,
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,

    /// CHECK: parsed by Chainlink SDK v2.
    pub chainlink_feed: UncheckedAccount<'info>,

    pub clock: Sysvar<'info, Clock>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CalcInternalValue<'info> {
    pub clock: Sysvar<'info, Clock>,
}
