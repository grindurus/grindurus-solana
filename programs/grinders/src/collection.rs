use anchor_lang::prelude::*;
use anchor_spl::metadata::{
    create_master_edition_v3, create_metadata_accounts_v3, verify_collection,
    mpl_token_metadata::types::{Collection, DataV2},
    CreateMasterEditionV3, CreateMetadataAccountsV3, VerifyCollection,
};
use anchor_spl::token::{self, MintTo};

/// ERC-721 collection name (`Grinders.sol` / `ERC721Enumerable`).
pub const COLLECTION_NAME: &str = "Grinders Custodians";
pub const COLLECTION_SYMBOL: &str = "GRINDERS";
/// IERC-1046 `tokenURI()` on EVM Grinders.
pub const COLLECTION_URI: &str = "https://grindurus.xyz/metadata.json";

pub const COLLECTION_MINT_SEED: &[u8] = b"collection";

pub fn collection_metadata_pda(collection_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"metadata",
            anchor_spl::metadata::ID.as_ref(),
            collection_mint.as_ref(),
        ],
        &anchor_spl::metadata::ID,
    )
}

pub fn collection_master_edition_pda(collection_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"metadata",
            anchor_spl::metadata::ID.as_ref(),
            collection_mint.as_ref(),
            b"edition",
        ],
        &anchor_spl::metadata::ID,
    )
}

pub fn custodian_metadata_pda(custodian_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"metadata",
            anchor_spl::metadata::ID.as_ref(),
            custodian_mint.as_ref(),
        ],
        &anchor_spl::metadata::ID,
    )
}

pub fn create_collection<'info>(
    grinders_state: &AccountInfo<'info>,
    grinders_bump: u8,
    collection_mint: &AccountInfo<'info>,
    collection_token_account: &AccountInfo<'info>,
    collection_metadata: &AccountInfo<'info>,
    collection_master_edition: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    token_metadata_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    rent: &AccountInfo<'info>,
) -> Result<()> {
    let signer_seeds: &[&[u8]] = &[crate::GrindersState::SEED, &[grinders_bump]];
    let signer = &[signer_seeds];

    create_metadata_accounts_v3(
        CpiContext::new_with_signer(
            token_metadata_program.clone(),
            CreateMetadataAccountsV3 {
                metadata: collection_metadata.clone(),
                mint: collection_mint.clone(),
                mint_authority: grinders_state.clone(),
                update_authority: grinders_state.clone(),
                payer: payer.clone(),
                system_program: system_program.clone(),
                rent: rent.clone(),
            },
            signer,
        ),
        DataV2 {
            name: COLLECTION_NAME.to_string(),
            symbol: COLLECTION_SYMBOL.to_string(),
            uri: COLLECTION_URI.to_string(),
            seller_fee_basis_points: 0,
            creators: None,
            collection: None,
            uses: None,
        },
        true,
        true,
        None,
    )?;

    token::mint_to(
        CpiContext::new_with_signer(
            token_program.clone(),
            MintTo {
                mint: collection_mint.clone(),
                to: collection_token_account.clone(),
                authority: grinders_state.clone(),
            },
            signer,
        ),
        1,
    )?;

    create_master_edition_v3(
        CpiContext::new_with_signer(
            token_metadata_program.clone(),
            CreateMasterEditionV3 {
                edition: collection_master_edition.clone(),
                mint: collection_mint.clone(),
                update_authority: grinders_state.clone(),
                mint_authority: grinders_state.clone(),
                payer: payer.clone(),
                metadata: collection_metadata.clone(),
                token_program: token_program.clone(),
                system_program: system_program.clone(),
                rent: rent.clone(),
            },
            signer,
        ),
        Some(0),
    )?;

    Ok(())
}

pub fn verify_custodian_collection<'info>(
    grinders_state: &AccountInfo<'info>,
    grinders_bump: u8,
    payer: &AccountInfo<'info>,
    custodian_metadata: &AccountInfo<'info>,
    collection_mint: &AccountInfo<'info>,
    collection_metadata: &AccountInfo<'info>,
    collection_master_edition: &AccountInfo<'info>,
    token_metadata_program: &AccountInfo<'info>,
) -> Result<()> {
    let signer_seeds: &[&[u8]] = &[crate::GrindersState::SEED, &[grinders_bump]];
    let signer = &[signer_seeds];

    verify_collection(
        CpiContext::new_with_signer(
            token_metadata_program.clone(),
            VerifyCollection {
                payer: payer.clone(),
                metadata: custodian_metadata.clone(),
                collection_authority: grinders_state.clone(),
                collection_mint: collection_mint.clone(),
                collection_metadata: collection_metadata.clone(),
                collection_master_edition: collection_master_edition.clone(),
            },
            signer,
        ),
        None,
    )?;
    Ok(())
}

pub fn collection_parent(collection_mint: &Pubkey) -> Collection {
    Collection {
        key: *collection_mint,
        verified: false,
    }
}
