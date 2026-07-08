use anchor_lang::prelude::*;
use anchor_lang::solana_program::ed25519_program;
use anchor_lang::solana_program::sysvar::instructions::{
    load_current_index_checked, load_instruction_at_checked,
};
use crate::{CustodyState, ErrorCode, SwapIntentData};

pub fn verify_owner_intent(
    custody: &Account<CustodyState>,
    intent: &SwapIntentData,
    instructions_sysvar: &AccountInfo,
) -> Result<()> {
    require!(
        custody.is_trading_mint(&intent.sell_mint),
        ErrorCode::InvalidSellMint
    );
    require!(
        custody.is_trading_mint(&intent.buy_mint),
        ErrorCode::InvalidBuyMint
    );
    require!(intent.sell_mint != intent.buy_mint, ErrorCode::InvalidSellMint);
    require!(intent.sell_amount > 0, ErrorCode::InvalidAmount);
    require!(intent.min_buy_amount > 0, ErrorCode::InvalidAmount);
    require!(intent.nonce == custody.swap_nonce, ErrorCode::InvalidSwapNonce);

    let clock = Clock::get()?;
    require!(
        clock.slot <= intent.expiry_slot,
        ErrorCode::SwapIntentExpired
    );

    let current_ix = load_current_index_checked(instructions_sysvar)? as usize;
    require!(current_ix > 0, ErrorCode::InvalidOwnerSignature);

    let ed25519_ix = load_instruction_at_checked(current_ix - 1, instructions_sysvar)?;
    require_keys_eq!(
        ed25519_ix.program_id,
        ed25519_program::id(),
        ErrorCode::InvalidOwnerSignature
    );

    let expected_message =
        intent.message_bytes(&custody.owner, custody.key());
    verify_ed25519_instruction_data(
        &ed25519_ix.data,
        &custody.owner.to_bytes(),
        &expected_message,
    )
}

fn verify_ed25519_instruction_data(
    data: &[u8],
    expected_pubkey: &[u8; 32],
    expected_message: &[u8],
) -> Result<()> {
    // Ed25519Program layout: u8 count, u8 padding, then offsets block (14 bytes), payload.
    require!(data.len() >= 16, ErrorCode::InvalidOwnerSignature);
    require!(data[0] == 1, ErrorCode::InvalidOwnerSignature);

    let signature_offset = u16::from_le_bytes([data[2], data[3]]) as usize;
    let pubkey_offset = u16::from_le_bytes([data[6], data[7]]) as usize;
    let message_offset = u16::from_le_bytes([data[10], data[11]]) as usize;
    let message_size = u16::from_le_bytes([data[12], data[13]]) as usize;

    require!(
        signature_offset + 64 <= data.len()
            && pubkey_offset + 32 <= data.len()
            && message_offset + message_size <= data.len(),
        ErrorCode::InvalidOwnerSignature
    );

    let signature = &data[signature_offset..signature_offset + 64];
    let pubkey = &data[pubkey_offset..pubkey_offset + 32];
    let message = &data[message_offset..message_offset + message_size];

    require!(pubkey == expected_pubkey, ErrorCode::InvalidOwnerSignature);
    require!(message == expected_message, ErrorCode::InvalidOwnerSignature);

  // Signature validity is enforced by the Ed25519 precompile in instruction 0.
    let _ = signature;
    Ok(())
}
