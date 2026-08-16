use crate::*;

#[event]
pub struct OFTSent {
    pub guid: [u8; 32],
    pub dst_eid: u32,
    pub from: Pubkey,
    pub amount_sent_ld: u64,
    pub amount_received_ld: u64,
}

#[event]
pub struct OFTReceived {
    pub guid: [u8; 32],
    pub src_eid: u32,
    pub to: Pubkey,
    pub amount_received_ld: u64,
}

#[event]
pub struct Vested {
    pub id: u64,
    pub from: Pubkey,
    pub to: Pubkey,
    pub amount_ld: u64,
}

#[event]
pub struct Released {
    pub id: u64,
    pub to: Pubkey,
    pub amount_ld: u64,
}

#[event]
pub struct SaleSet {
    pub id: u64,
    pub asset: Pubkey,
    pub asset_amount: u64,
    pub recipient: Pubkey,
    pub grs_amount: u64,
}

#[event]
pub struct SaleAccepted {
    pub id: u64,
    pub asset: Pubkey,
    pub asset_amount: u64,
    pub recipient: Pubkey,
    pub grs_amount: u64,
}

#[event]
pub struct SalePublished {
    pub id: u64,
    pub dst_eid: u32,
    pub guid: [u8; 32],
}

#[event]
pub struct Bought {
    pub id: u64,
    pub buyer: Pubkey,
    pub to: Pubkey,
    pub amount_ld: u64,
    pub cost: u64,
}
