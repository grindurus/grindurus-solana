use anchor_lang::prelude::*;

pub const SWAP_INTENT_PREFIX: &[u8] = b"GRINDURUS_CUSTODY_SWAP_V1";
pub const DISABLE_DELAY_SECONDS: i64 = 24 * 60 * 60;
pub const NATIVE_SOL_MINT: Pubkey = anchor_lang::solana_program::system_program::ID;

#[account]
pub struct CustodyState {
    pub owner: Pubkey,
    pub grai_program: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub grinder_id: u64,
    pub swap_nonce: u64,
    pub emergency_withdraw_disabled: bool,
    pub emergency_withdraw_scheduled_at: i64,
    pub bump: u8,
}

impl CustodyState {
    pub const SEED: &'static [u8] = b"custody";
    pub const LEN: usize = 32 + 32 + 32 + 32 + 8 + 8 + 1 + 8 + 1;

    pub fn custody_signer_seeds<'a>(
        owner: &'a Pubkey,
        grinder_id_bytes: &'a [u8; 8],
        bump: &'a [u8; 1],
    ) -> [&'a [u8]; 4] {
        [Self::SEED, owner.as_ref(), grinder_id_bytes, bump]
    }

    pub fn is_trading_mint(&self, mint: &Pubkey) -> bool {
        *mint == self.base_mint || *mint == self.quote_mint || *mint == NATIVE_SOL_MINT
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SwapIntentData {
    pub nonce: u64,
    pub sell_mint: Pubkey,
    pub buy_mint: Pubkey,
    pub sell_amount: u64,
    pub min_buy_amount: u64,
    pub expiry_slot: u64,
}

impl SwapIntentData {
    pub fn message_bytes(&self, owner: &Pubkey, custody: Pubkey) -> Vec<u8> {
        let mut message = Vec::with_capacity(
            SWAP_INTENT_PREFIX.len() + 32 + 32 + 8 + 32 + 32 + 8 + 8 + 8,
        );
        message.extend_from_slice(SWAP_INTENT_PREFIX);
        message.extend_from_slice(owner.as_ref());
        message.extend_from_slice(custody.as_ref());
        message.extend_from_slice(&self.nonce.to_le_bytes());
        message.extend_from_slice(self.sell_mint.as_ref());
        message.extend_from_slice(self.buy_mint.as_ref());
        message.extend_from_slice(&self.sell_amount.to_le_bytes());
        message.extend_from_slice(&self.min_buy_amount.to_le_bytes());
        message.extend_from_slice(&self.expiry_slot.to_le_bytes());
        message
    }
}
