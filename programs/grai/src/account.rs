use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::Metadata;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::price_feed;
use crate::{AssetConfig, ErrorCode, GraiState, VoteEscrow, YieldBy};

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = GraiState::space(0, 0),
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
pub struct SetGrinders<'info> {
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
pub struct SetProtocolConfig<'info> {
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
pub struct SetSettlementAsset<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    pub settlement_mint: Account<'info, Mint>,

    #[account(
        seeds = [AssetConfig::SEED, settlement_mint.key().as_ref()],
        bump = settlement_asset_config.bump,
        constraint = settlement_asset_config.asset_mint == settlement_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub settlement_asset_config: Account<'info, AssetConfig>,

    /// CHECK: Price feed for the new settlement asset.
    #[account(
        constraint = settlement_price_feed.key() == settlement_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&settlement_price_feed.to_account_info(), settlement_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub settlement_price_feed: UncheckedAccount<'info>,

    /// Previous settlement mint (may equal settlement_mint on first set).
    pub previous_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, previous_mint.key().as_ref()],
        bump = previous_asset_config.bump,
        constraint = previous_asset_config.asset_mint == previous_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub previous_asset_config: Account<'info, AssetConfig>,

    /// CHECK: Price feed for previous settlement (used if inventory must be put to auction).
    #[account(
        constraint = previous_price_feed.key() == previous_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub previous_price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, previous_mint.key().as_ref()],
        bump,
        constraint = previous_vault_ata.mint == previous_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub previous_vault_ata: Account<'info, TokenAccount>,
}

#[derive(Accounts)]
pub struct AddAsset<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
        realloc = GraiState::space(grai_state.asset_mints.len() + 1, grai_state.voters.len()),
        realloc::payer = authority,
        realloc::zero = false,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        init,
        payer = authority,
        space = 8 + AssetConfig::LEN,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub asset_config: Account<'info, AssetConfig>,

    #[account(
        init_if_needed,
        payer = authority,
        token::mint = asset_mint,
        token::authority = grai_state,
        seeds = [AssetConfig::VAULT_SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub vault_ata: Account<'info, TokenAccount>,

    /// CHECK: Chainlink, Pyth, or custom price feed for `asset_mint`.
    #[account(
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
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
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Account<'info, AssetConfig>,

    /// CHECK: New price feed for `asset_mint`.
    #[account(
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct SetAssetConfig<'info> {
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
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Account<'info, AssetConfig>,
}

#[derive(Accounts)]
pub struct RemoveAsset<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
        realloc = GraiState::space(grai_state.asset_mints.len().saturating_sub(1), grai_state.voters.len()),
        realloc::payer = authority,
        realloc::zero = false,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        close = authority,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Account<'info, AssetConfig>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = vault_ata.amount == 0 @ ErrorCode::AssetBalanceNonZero,
    )]
    pub vault_ata: Account<'info, TokenAccount>,

    /// CHECK: Optional moved asset config when swapping list indices — validated in handler if needed.
    /// Pass `system_program` as a dummy when unused (last asset / no swap).
    pub moved_asset_config: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Price feed for deposit asset.
    #[account(
        constraint = price_feed.key() == asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    /// CHECK: Grinders state PDA — must match `grai_state.grinders`.
    #[account(
        constraint = grinders_state.key() == grai_state.grinders @ ErrorCode::InvalidGrinders,
    )]
    pub grinders_state: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = depositor_ata.mint == asset_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = depositor_ata.owner == depositor.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub depositor_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = asset_mint,
        associated_token::authority = grinders_state,
    )]
    pub grinders_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = grai_mint,
        associated_token::authority = depositor,
    )]
    pub depositor_grai_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositSol<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
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

    #[account(
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Price feed for WSOL.
    #[account(
        constraint = price_feed.key() == asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    /// CHECK: Grinders state PDA — must match `grai_state.grinders`.
    #[account(
        constraint = grinders_state.key() == grai_state.grinders @ ErrorCode::InvalidGrinders,
    )]
    pub grinders_state: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = asset_mint,
        associated_token::authority = depositor,
    )]
    pub depositor_wsol_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = asset_mint,
        associated_token::authority = grinders_state,
    )]
    pub grinders_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = grai_mint,
        associated_token::authority = depositor,
    )]
    pub depositor_grai_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Distribute<'info> {
    #[account(mut)]
    pub custody_wallet: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Price feed for yield asset.
    #[account(
        constraint = price_feed.key() == asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    pub settlement_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, settlement_mint.key().as_ref()],
        bump = settlement_asset_config.bump,
        constraint = settlement_asset_config.asset_mint == settlement_mint.key() @ ErrorCode::AssetUnknown,
        constraint = settlement_mint.key() == grai_state.settlement_asset @ ErrorCode::SettlementAssetUnset,
    )]
    pub settlement_asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Price feed for settlement asset.
    #[account(
        constraint = settlement_price_feed.key() == settlement_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub settlement_price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = custody_ata.mint == asset_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = custody_ata.owner == custody_wallet.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub custody_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = treasury_ata.owner == grai_state.treasury @ ErrorCode::InvalidDestination,
    )]
    pub treasury_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + YieldBy::LEN,
        seeds = [YieldBy::SEED, custody_wallet.key().as_ref(), asset_mint.key().as_ref()],
        bump,
    )]
    pub yield_by: Account<'info, YieldBy>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Fill<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    #[account(
        constraint = settlement_mint.key() == grai_state.settlement_asset @ ErrorCode::SettlementAssetUnset,
    )]
    pub settlement_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, settlement_mint.key().as_ref()],
        bump,
        constraint = settlement_vault_ata.mint == settlement_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub settlement_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = buyer_settlement_ata.mint == settlement_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = buyer_settlement_ata.owner == buyer.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub buyer_settlement_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = buyer,
        associated_token::mint = asset_mint,
        associated_token::authority = buyer,
    )]
    pub buyer_asset_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Vote<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Account<'info, Mint>,

    #[account(
        init_if_needed,
        payer = voter,
        space = 8 + VoteEscrow::LEN,
        seeds = [VoteEscrow::SEED, voter.key().as_ref()],
        bump,
    )]
    pub vote_escrow: Account<'info, VoteEscrow>,

    #[account(
        mut,
        constraint = voter_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = voter_grai_ata.owner == voter.key() @ ErrorCode::InvalidDestination,
    )]
    pub voter_grai_ata: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = voter,
        token::mint = grai_mint,
        token::authority = grai_state,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
    )]
    pub grai_vault_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Bribe<'info> {
    #[account(mut)]
    pub briber: Signer<'info>,

    /// CHECK: Voter whose escrow is bought out.
    pub voter: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [VoteEscrow::SEED, voter.key().as_ref()],
        bump = vote_escrow.bump,
    )]
    pub vote_escrow: Account<'info, VoteEscrow>,

    pub settlement_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, settlement_mint.key().as_ref()],
        bump = settlement_asset_config.bump,
        constraint = settlement_mint.key() == grai_state.settlement_asset @ ErrorCode::SettlementAssetUnset,
    )]
    pub settlement_asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Settlement price feed.
    #[account(
        constraint = settlement_price_feed.key() == settlement_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub settlement_price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, settlement_mint.key().as_ref()],
        bump,
        constraint = settlement_vault_ata.mint == settlement_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub settlement_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = briber,
        associated_token::mint = grai_mint,
        associated_token::authority = briber,
    )]
    pub briber_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = briber,
        associated_token::mint = grai_mint,
        associated_token::authority = voter,
    )]
    pub voter_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = briber_settlement_ata.mint == settlement_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = briber_settlement_ata.owner == briber.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub briber_settlement_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = briber,
        associated_token::mint = settlement_mint,
        associated_token::authority = voter,
    )]
    pub voter_settlement_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury_settlement_ata.mint == settlement_mint.key() @ ErrorCode::InvalidDestination,
        constraint = treasury_settlement_ata.owner == grai_state.treasury @ ErrorCode::InvalidDestination,
    )]
    pub treasury_settlement_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Resolve<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Liquidate<'info> {
    #[account(mut)]
    pub holder: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = holder,
        space = 8 + VoteEscrow::LEN,
        seeds = [VoteEscrow::SEED, holder.key().as_ref()],
        bump,
    )]
    pub vote_escrow: Account<'info, VoteEscrow>,

    #[account(
        mut,
        constraint = holder_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = holder_grai_ata.owner == holder.key() @ ErrorCode::InvalidDestination,
    )]
    pub holder_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Buyback<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    /// CHECK: Grinders program executable.
    #[account(
        executable,
        address = crate::buyback::GRINDERS_PROGRAM_ID @ ErrorCode::InvalidGrinders,
    )]
    pub grinders_program: UncheckedAccount<'info>,

    /// CHECK: Grinders state PDA configured on GRAI.
    #[account(
        address = grai_state.grinders @ ErrorCode::InvalidGrinders,
    )]
    pub grinders_state: UncheckedAccount<'info>,

    #[account(
        constraint = settlement_mint.key() == grai_state.settlement_asset @ ErrorCode::SettlementAssetUnset,
    )]
    pub settlement_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, settlement_mint.key().as_ref()],
        bump,
        constraint = settlement_vault_ata.mint == settlement_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub settlement_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = settlement_mint,
        associated_token::authority = grinders_state,
    )]
    pub grinders_settlement_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Account<'info, Mint>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = grai_mint,
        associated_token::authority = grinders_state,
    )]
    pub grai_grinders_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct GetAssets<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct HasQuorum<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Account<'info, Mint>,
}
