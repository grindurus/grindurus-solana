use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;

/// Sentinel for native SOL withdrawals (mirrors EVM `address(0)`).
pub const NATIVE_ASSET: Pubkey = Pubkey::new_from_array([0u8; 32]);

/// CAIP Label namespace — matches EVM `Custodian._labelName()` base.
pub const LABEL_NAMESPACE: &str = "grinder.custodian";

/// CAIP-2 namespace for Solana.
pub const CAIP2_SOLANA: &str = "solana";

/// Adapter local name for router CPI swaps (EVM `SwapCustodian` → `grinder.custodian.swap`).
pub const LABEL_NAME_SWAP: &str = "swap";

/// Adapter local name for Jupiter gasless path.
pub const LABEL_NAME_JUPITER_GASLESS: &str = "jupiter_gasless";

#[account]
#[derive(InitSpace)]
pub struct GrindersState {
    pub owner: Pubkey,
    /// Two-step handoff target (EVM `Ownable2Step.pendingOwner`). Default = none.
    pub pending_owner: Pubkey,
    pub grai_program: Pubkey,
    pub next_custodian_id: u64,
    /// Metaplex collection parent for all custodian NFTs (mirrors ERC-721 contract).
    pub collection_mint: Pubkey,
    /// Last successful operational heartbeat (`allocate` / `custodian_deallocate` / `custodian_distribute` / `heartbeat`).
    pub heartbeat_at: i64,
    /// Inactivity window in seconds. If `now > heartbeat_at + grinding_period`, Grinders is stale.
    pub grinding_period: u32,
    pub bump: u8,
    /// CAIP-2 reference (utf8, null-padded): genesis hash or `localnet`.
    /// Kept after `bump` so GRAI raw heartbeat offsets stay stable.
    pub cluster_ref: [u8; 32],
}

impl GrindersState {
    pub const SEED: &'static [u8] = b"grinders";
    /// Borsh body (no discriminator).
    /// `owner + pending + grai + collection + next_id + heartbeat_at + grinding_period + bump + cluster_ref`
    /// = `32*4 + 8 + 8 + 4 + 1 + 32` = 181.
    pub const LEN: usize = Self::INIT_SPACE;

    pub fn signer_seeds<'a>(&'a self, bump: &'a [u8; 1]) -> [&'a [u8]; 2] {
        [Self::SEED, bump]
    }

    pub fn cluster_ref_str(&self) -> &str {
        cluster_ref_str(&self.cluster_ref)
    }
}

/// On-chain custodian wallet PDA — custodian authority + registry (former `CustodianRecord`).
#[account]
#[derive(InitSpace)]
pub struct CustodianState {
    pub grinders: Pubkey,
    pub custodian_id: u64,
    pub grai_program: Pubkey,
    /// `keccak256(utf8(labelId))` — same as EVM `Custodian.label()`.
    pub label: [u8; 32],
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub nft_mint: Pubkey,
    /// Cached holder hint (updated on mint / `transfer_custodian_nft`). Auth uses live SPL ATA.
    pub nft_owner: Pubkey,
    pub bump: u8,
}

impl CustodianState {
    pub const SEED: &'static [u8] = b"custodian_wallet";
    pub const LEN: usize = Self::INIT_SPACE;

    pub fn signer_seeds<'a>(
        grinders: &'a [u8],
        custodian_id_bytes: &'a [u8; 8],
        bump: &'a [u8; 1],
    ) -> [&'a [u8]; 4] {
        [Self::SEED, grinders, custodian_id_bytes, bump]
    }
}

/// Trim null-padded utf8 CAIP-2 reference.
pub fn cluster_ref_str(bytes: &[u8; 32]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(32);
    core::str::from_utf8(&bytes[..end]).unwrap_or("")
}

/// Encode a null-padded cluster reference (max 32 bytes).
pub fn encode_cluster_ref(s: &str) -> Result<[u8; 32]> {
    let b = s.as_bytes();
    require!(!b.is_empty() && b.len() <= 32, crate::errors::ErrorCode::InvalidClusterRef);
    require!(
        b.iter().all(|&c| c.is_ascii_graphic()),
        crate::errors::ErrorCode::InvalidClusterRef
    );
    let mut out = [0u8; 32];
    out[..b.len()].copy_from_slice(b);
    Ok(out)
}

/// CAIP Label ID: `{namespace}.{adapter}@solana:{cluster_ref}`.
pub fn label_id(adapter: &str, cluster_ref: &str) -> String {
    if adapter.contains('@') {
        return adapter.to_string();
    }
    let label_name = if adapter.starts_with(LABEL_NAMESPACE) {
        adapter.to_string()
    } else {
        format!("{LABEL_NAMESPACE}.{adapter}")
    };
    format!("{label_name}@{CAIP2_SOLANA}:{cluster_ref}")
}

/// `keccak256(utf8(label_id))` — EVM `Custodian.label()`.
pub fn label_hash(adapter: &str, cluster_ref: &str) -> [u8; 32] {
    keccak::hash(label_id(adapter, cluster_ref).as_bytes()).0
}

pub fn swap_label(cluster_ref: &[u8; 32]) -> [u8; 32] {
    label_hash(LABEL_NAME_SWAP, cluster_ref_str(cluster_ref))
}

pub fn jupiter_gasless_label(cluster_ref: &[u8; 32]) -> [u8; 32] {
    label_hash(LABEL_NAME_JUPITER_GASLESS, cluster_ref_str(cluster_ref))
}

pub fn is_known_label(label: &[u8; 32], cluster_ref: &[u8; 32]) -> bool {
    *label == swap_label(cluster_ref) || *label == jupiter_gasless_label(cluster_ref)
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

#[cfg(test)]
mod account_sizes {
    use super::*;
    use anchor_lang::AccountSerialize;

    #[test]
    fn grinders_state_len_matches_borsh() {
        let state = GrindersState {
            owner: Pubkey::new_unique(),
            pending_owner: Pubkey::default(),
            grai_program: Pubkey::new_unique(),
            next_custodian_id: 1,
            collection_mint: Pubkey::new_unique(),
            heartbeat_at: 1,
            grinding_period: 604_800,
            bump: 255,
            cluster_ref: encode_cluster_ref("localnet").unwrap(),
        };
        let mut buf = Vec::new();
        state.try_serialize(&mut buf).unwrap();
        let expected = 32 * 4 + 8 + 8 + 4 + 1 + 32;
        assert_eq!(buf.len(), 8 + expected);
        assert_eq!(GrindersState::LEN, expected);
    }

    #[test]
    fn custodian_state_len_matches_borsh() {
        let state = CustodianState {
            grinders: Pubkey::new_unique(),
            custodian_id: 0,
            grai_program: Pubkey::new_unique(),
            label: [0; 32],
            base_mint: Pubkey::default(),
            quote_mint: Pubkey::default(),
            nft_mint: Pubkey::new_unique(),
            nft_owner: Pubkey::new_unique(),
            bump: 1,
        };
        let mut buf = Vec::new();
        state.try_serialize(&mut buf).unwrap();
        assert_eq!(buf.len(), 8 + CustodianState::LEN);
    }

    #[test]
    fn label_hash_localnet_swap() {
        // keccak256("grinder.custodian.swap@solana:localnet")
        let expected: [u8; 32] = [
            0x34, 0x64, 0xb1, 0x50, 0x32, 0xb5, 0xbe, 0x79, 0x3d, 0x52, 0xb4, 0x16, 0x59, 0xce,
            0x9e, 0x07, 0xf0, 0x59, 0x24, 0x17, 0x8e, 0x0d, 0x71, 0x02, 0x0e, 0x9e, 0xe0, 0x79,
            0x22, 0xd6, 0xb9, 0x34,
        ];
        assert_eq!(label_hash("swap", "localnet"), expected);
    }
}
