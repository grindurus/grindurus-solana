use anchor_lang::prelude::*;
use anchor_spl::metadata::{
    create_master_edition_v3, create_metadata_accounts_v3,
    mpl_token_metadata::types::{Creator, DataV2},
    CreateMasterEditionV3, CreateMetadataAccountsV3,
};
use anchor_spl::token::{self, MintTo};

use crate::GraiState;

pub const TOKEN_NAME: &str = "Grinders Artificial Index";
pub const TOKEN_SYMBOL: &str = "GRAI";
pub const TOKEN_URI: &str = "https://grindurus.xyz/grai.json";

pub const TREASURY_NFT_NAME: &str = "Treasury";
pub const TREASURY_NFT_SYMBOL: &str = "T-GRAI";
pub const TREASURY_NFT_URI: &str = "https://grindurus.xyz/treasury.json";
pub const TREASURY_NFT_SEED: &[u8] = b"treasury-nft";

pub fn create_grai_metadata<'info>(
    metadata: AccountInfo<'info>,
    grai_mint: AccountInfo<'info>,
    grai_state: AccountInfo<'info>,
    payer: AccountInfo<'info>,
    token_metadata_program: AccountInfo<'info>,
    system_program: AccountInfo<'info>,
    rent: AccountInfo<'info>,
    grai_state_bump: u8,
) -> Result<()> {
    let seeds: &[&[u8]; 2] = &[GraiState::SEED, &[grai_state_bump]];
    let signer: &[&[&[u8]]; 1] = &[&seeds[..]];

    create_metadata_accounts_v3(
        CpiContext::new_with_signer(
            token_metadata_program,
            CreateMetadataAccountsV3 {
                metadata,
                mint: grai_mint,
                mint_authority: grai_state.clone(),
                update_authority: grai_state,
                payer,
                system_program,
                rent,
            },
            signer,
        ),
        DataV2 {
            name: TOKEN_NAME.to_string(),
            symbol: TOKEN_SYMBOL.to_string(),
            uri: TOKEN_URI.to_string(),
            seller_fee_basis_points: 0,
            creators: None,
            collection: None,
            uses: None,
        },
        true,
        true,
        None,
    )?;

    Ok(())
}

/// Mint a 1/1 Metaplex NFT representing EVM Treasury ERC-721 cashflow rights for `locker`.
///
/// `mint` must be the PDA `["treasury-nft", locker]` with decimals 0, mint authority =
/// `GraiState`. Mints exactly one token to `nft_ata`, then locks supply via Master Edition.
#[allow(clippy::too_many_arguments)]
pub fn mint_treasury_nft<'info>(
    metadata: AccountInfo<'info>,
    master_edition: AccountInfo<'info>,
    mint: AccountInfo<'info>,
    nft_ata: AccountInfo<'info>,
    grai_state: AccountInfo<'info>,
    payer: AccountInfo<'info>,
    token_metadata_program: AccountInfo<'info>,
    token_program: AccountInfo<'info>,
    system_program: AccountInfo<'info>,
    rent: AccountInfo<'info>,
    grai_state_bump: u8,
    royalty_bps: u16,
    royalty_payee: Pubkey,
) -> Result<()> {
    let seeds: &[&[u8]; 2] = &[GraiState::SEED, &[grai_state_bump]];
    let signer: &[&[&[u8]]; 1] = &[&seeds[..]];

    token::mint_to(
        CpiContext::new_with_signer(
            token_program.clone(),
            MintTo {
                mint: mint.clone(),
                to: nft_ata,
                authority: grai_state.clone(),
            },
            signer,
        ),
        1,
    )?;

    create_metadata_accounts_v3(
        CpiContext::new_with_signer(
            token_metadata_program.clone(),
            CreateMetadataAccountsV3 {
                metadata: metadata.clone(),
                mint: mint.clone(),
                mint_authority: grai_state.clone(),
                update_authority: grai_state.clone(),
                payer: payer.clone(),
                system_program: system_program.clone(),
                rent: rent.clone(),
            },
            signer,
        ),
        DataV2 {
            name: TREASURY_NFT_NAME.to_string(),
            symbol: TREASURY_NFT_SYMBOL.to_string(),
            uri: TREASURY_NFT_URI.to_string(),
            seller_fee_basis_points: royalty_bps,
            creators: treasury_royalty_creators(royalty_payee, grai_state.key()),
            collection: None,
            uses: None,
        },
        true,
        true,
        None,
    )?;

    create_master_edition_v3(
        CpiContext::new_with_signer(
            token_metadata_program,
            CreateMasterEditionV3 {
                edition: master_edition,
                mint,
                update_authority: grai_state.clone(),
                mint_authority: grai_state,
                payer,
                metadata,
                token_program,
                system_program,
                rent,
            },
            signer,
        ),
        Some(0),
    )?;

    Ok(())
}

/// `beneficiar` (or owner) gets 100% of `seller_fee_basis_points` on secondary sales.
///
/// `verified` is true only when the payee is the metadata update authority (`GraiState`),
/// which signs this CPI. A wallet beneficiar cannot be verified at mint — they do not
/// sign `deposit` — so marketplaces that require verified creators may skip the fee.
/// Creator is snapshotted at mint; `set_beneficiar` does not rewrite existing metadata.
fn treasury_royalty_creators(payee: Pubkey, update_authority: Pubkey) -> Option<Vec<Creator>> {
    Some(vec![Creator {
        address: payee,
        verified: payee == update_authority,
        share: 100,
    }])
}
