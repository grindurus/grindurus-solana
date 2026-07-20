use anchor_lang::prelude::*;

/// Sentinel for native SOL withdrawals (mirrors EVM `address(0)`).
pub const NATIVE_ASSET: Pubkey = Pubkey::new_from_array([0u8; 32]);

#[account]
pub struct GrindersState {
    pub owner: Pubkey,
    pub grai_program: Pubkey,
    pub next_custodian_id: u64,
    /// Metaplex collection parent for all custodian NFTs (mirrors ERC-721 contract).
    pub collection_mint: Pubkey,
    pub bump: u8,
}

impl GrindersState {
    pub const SEED: &'static [u8] = b"grinders";
    pub const LEN: usize = 32 + 32 + 8 + 32 + 1;

    pub fn signer_seeds<'a>(&'a self, bump: &'a [u8; 1]) -> [&'a [u8]; 2] {
        [Self::SEED, bump]
    }
}

#[account]
pub struct CustodianRecord {
    pub custodian_id: u64,
    pub custodian_wallet: Pubkey,
    pub nft_mint: Pubkey,
    pub nft_owner: Pubkey,
    pub custodian_kind: [u8; 32],
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub bump: u8,
}

impl CustodianRecord {
    pub const SEED: &'static [u8] = b"custodian";
    pub const LEN: usize = 8 + 32 + 32 + 32 + 32 + 32 + 32 + 1;

    pub fn signer_seeds<'a>(
        custodian_id_bytes: &'a [u8; 8],
        bump: &'a [u8; 1],
    ) -> [&'a [u8]; 3] {
        [Self::SEED, custodian_id_bytes, bump]
    }
}

#[account]
pub struct CustodianIndex {
    pub custodian_id: u64,
    pub bump: u8,
}

impl CustodianIndex {
    pub const SEED: &'static [u8] = b"custodian_index";
    pub const LEN: usize = 8 + 1;
}

/// On-chain swap custodian wallet PDA (mirrors `SwapCustodian.sol` proxy address).
#[account]
pub struct CustodianState {
    pub grinders: Pubkey,
    pub custodian_id: u64,
    pub grai_program: Pubkey,
    /// `keccak256("grindurus.custodian.<name>")` — same as EVM `custodianKind()`.
    pub custodian_kind: [u8; 32],
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub bump: u8,
}

impl CustodianState {
    pub const SEED: &'static [u8] = b"custodian_wallet";
    pub const LEN: usize = 32 + 8 + 32 + 32 + 32 + 32 + 1;

    pub fn signer_seeds<'a>(
        grinders: &'a [u8],
        custodian_id_bytes: &'a [u8; 8],
        bump: &'a [u8; 1],
    ) -> [&'a [u8]; 4] {
        [Self::SEED, grinders, custodian_id_bytes, bump]
    }
}

/// keccak256("grindurus.custodian.explicit_swap")
pub const EXPLICIT_SWAP_CUSTODIAN_KIND: [u8; 32] = [
    0xed, 0x40, 0x2d, 0x39, 0xd1, 0x7f, 0xde, 0x1c, 0xee, 0x54, 0x97, 0xb1, 0x83, 0x6d, 0xb0, 0x76,
    0x72, 0x1a, 0xee, 0xd0, 0x7c, 0x63, 0x37, 0xad, 0x6f, 0x98, 0x15, 0x59, 0xe6, 0x93, 0x83, 0xad,
];

/// keccak256("grindurus.custodian.jupiter_gasless")
pub const JUPITER_GASLESS_CUSTODIAN_KIND: [u8; 32] = [
    0xab, 0x8d, 0xfa, 0x36, 0xd3, 0x32, 0xa5, 0xed, 0x58, 0x3f, 0x12, 0xdc, 0xba, 0xa9, 0x4c, 0x95,
    0x1a, 0x77, 0x72, 0x1b, 0xa1, 0xa2, 0x57, 0x97, 0xe8, 0x15, 0x18, 0x79, 0x51, 0x04, 0x0a, 0x02,
];

pub fn is_known_custodian_kind(kind: &[u8; 32]) -> bool {
    *kind == EXPLICIT_SWAP_CUSTODIAN_KIND || *kind == JUPITER_GASLESS_CUSTODIAN_KIND
}

pub fn custodian_state_pda(grinders: &Pubkey, custodian_id: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            CustodianState::SEED,
            grinders.as_ref(),
            &custodian_id.to_le_bytes(),
        ],
        &crate::ID,
    )
}

/// Per-custodian issuance ledger for an asset (mirrors EVM `Grinders.allocated`).
#[account]
pub struct Allocation {
    pub allocated_amount: u64,
    pub bump: u8,
}

impl Allocation {
    pub const SEED: &'static [u8] = b"allocation";
    pub const LEN: usize = 8 + 1;
}
