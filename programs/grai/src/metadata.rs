use anchor_lang::prelude::*;
use anchor_spl::metadata::{
    create_metadata_accounts_v3, mpl_token_metadata::types::DataV2, CreateMetadataAccountsV3,
};

use crate::MintConfig;

pub const TOKEN_NAME: &str = "Grinders Artificial Index";
pub const TOKEN_SYMBOL: &str = "GRAI";
pub const TOKEN_URI: &str = "https://grindurus.xyz/metadata.json";

pub fn create_grai_metadata<'info>(
    metadata: AccountInfo<'info>,
    grai_mint: AccountInfo<'info>,
    mint_config: AccountInfo<'info>,
    payer: AccountInfo<'info>,
    token_metadata_program: AccountInfo<'info>,
    system_program: AccountInfo<'info>,
    rent: AccountInfo<'info>,
    mint_config_bump: u8,
) -> Result<()> {
    let seeds: &[&[u8]; 2] = &[MintConfig::SEED, &[mint_config_bump]];
    let signer: &[&[&[u8]]; 1] = &[&seeds[..]];

    create_metadata_accounts_v3(
        CpiContext::new_with_signer(
            token_metadata_program,
            CreateMetadataAccountsV3 {
                metadata,
                mint: grai_mint,
                mint_authority: mint_config.clone(),
                update_authority: mint_config,
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

    msg!(
        "GRAI metadata created: name={}, symbol={}",
        TOKEN_NAME,
        TOKEN_SYMBOL
    );
    Ok(())
}
