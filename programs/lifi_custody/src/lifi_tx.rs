use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::{invoke, invoke_signed};

use crate::ErrorCode;

pub fn execute_routed_swap_instructions<'info>(
    custody: &AccountInfo<'info>,
    custody_signer_seeds: &[&[&[u8]]],
    remaining_accounts: &[AccountInfo<'info>],
    instruction_data: &[Vec<u8>],
    account_counts: &[u8],
) -> Result<()> {
    require!(
        !instruction_data.is_empty(),
        ErrorCode::InvalidLifiTransaction
    );
    require!(
        instruction_data.len() == account_counts.len(),
        ErrorCode::InvalidLifiTransaction
    );

    let mut offset = 0usize;
    for (ix_data, account_count) in instruction_data.iter().zip(account_counts.iter()) {
        let count = *account_count as usize;
        require!(count > 0, ErrorCode::InvalidLifiAccounts);
        require!(
            offset + count <= remaining_accounts.len(),
            ErrorCode::InvalidLifiAccounts
        );

        let slice = &remaining_accounts[offset..offset + count];
        let program_info = &slice[0];
        let metas: Vec<AccountMeta> = slice[1..]
            .iter()
            .map(|account| AccountMeta {
                pubkey: *account.key,
                is_signer: account.is_signer,
                is_writable: account.is_writable,
            })
            .collect();

        let ix = Instruction {
            program_id: *program_info.key,
            accounts: metas,
            data: ix_data.clone(),
        };

        let needs_custody_signer = slice[1..]
            .iter()
            .any(|account| account.is_signer && account.key == custody.key);

        if needs_custody_signer {
            invoke_signed(&ix, slice, custody_signer_seeds)?;
        } else {
            invoke(&ix, slice)?;
        }

        offset += count;
    }

    require!(
        offset == remaining_accounts.len(),
        ErrorCode::InvalidLifiAccounts
    );
    Ok(())
}
