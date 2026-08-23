use crate::*;
use anchor_lang::system_program::{self, Transfer};

/// Resize / rewrite `LzReceiveTypesAccounts` to include `grs_config` (flat pubkey list for the Executor).
/// Idempotent when already patched. Uses `UncheckedAccount` so pre-patch (2-pubkey) data still loads.
#[derive(Accounts)]
pub struct PatchLzReceiveTypesAccounts<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        has_one = admin @ OFTError::Unauthorized
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump
    )]
    pub grs_config: Account<'info, GrsConfig>,
    /// CHECK: PDA `["LzReceiveTypes", oft_store]`; may still be the old 2-pubkey layout.
    #[account(
        mut,
        seeds = [LZ_RECEIVE_TYPES_SEED, oft_store.key().as_ref()],
        bump,
    )]
    pub lz_receive_types_accounts: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

impl PatchLzReceiveTypesAccounts<'_> {
    pub fn apply(ctx: &Context<PatchLzReceiveTypesAccounts>) -> Result<()> {
        let new_len = 8 + LzReceiveTypesAccounts::INIT_SPACE;
        let ai = ctx.accounts.lz_receive_types_accounts.to_account_info();
        let rent = Rent::get()?.minimum_balance(new_len);
        let current_lamports = ai.lamports();
        if rent > current_lamports {
            system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.admin.to_account_info(),
                        to: ai.clone(),
                    },
                ),
                rent - current_lamports,
            )?;
        }
        if ai.data_len() != new_len {
            ai.realloc(new_len, false)?;
        }

        let state = LzReceiveTypesAccounts {
            oft_store: ctx.accounts.oft_store.key(),
            token_mint: ctx.accounts.oft_store.token_mint,
            grs_config: ctx.accounts.grs_config.key(),
        };
        let mut data = ai.try_borrow_mut_data()?;
        let mut cursor: &mut [u8] = &mut data;
        state.try_serialize(&mut cursor)?;
        Ok(())
    }
}
