use anchor_lang::prelude::*;
use anchor_spl::metadata::{
    create_metadata_accounts_v3, mpl_token_metadata::types::DataV2, CreateMetadataAccountsV3,
};

/// On-chain Metaplex fields for wallets / explorers (parity with EVM `OFT("GrindURUS Token", "GRS")`).
pub const TOKEN_NAME: &str = "GrindURUS Token";
pub const TOKEN_SYMBOL: &str = "GRS";
pub const TOKEN_URI: &str = "https://grindurus.xyz/grs.json";

/// Create Metaplex Token Metadata for the GRS mint. `mint_authority` must sign (admin or OFT PDA).
pub fn create_grs_metadata<'info>(
    metadata: AccountInfo<'info>,
    token_mint: AccountInfo<'info>,
    mint_authority: AccountInfo<'info>,
    update_authority: AccountInfo<'info>,
    payer: AccountInfo<'info>,
    token_metadata_program: AccountInfo<'info>,
    system_program: AccountInfo<'info>,
    rent: AccountInfo<'info>,
    mint_authority_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    let accounts = CreateMetadataAccountsV3 {
        metadata,
        mint: token_mint,
        mint_authority,
        update_authority,
        payer,
        system_program,
        rent,
    };
    let data = DataV2 {
        name: TOKEN_NAME.to_string(),
        symbol: TOKEN_SYMBOL.to_string(),
        uri: TOKEN_URI.to_string(),
        seller_fee_basis_points: 0,
        creators: None,
        collection: None,
        uses: None,
    };
    let ctx = if let Some(seeds) = mint_authority_seeds {
        CpiContext::new_with_signer(token_metadata_program, accounts, seeds)
    } else {
        CpiContext::new(token_metadata_program, accounts)
    };
    create_metadata_accounts_v3(ctx, data, true, true, None)?;
    Ok(())
}
