use crate::*;

pub const GRS_MAX_PEERS: usize = 16;

/// Enumerable LayerZero peers for this OFT (mirrors EVM `GRS.getPeers`).
#[account]
#[derive(InitSpace)]
pub struct PeerRegistry {
    pub oft_store: Pubkey,
    pub bump: u8,
    #[max_len(GRS_MAX_PEERS)]
    pub entries: Vec<PeerEntry>,
}

#[derive(Clone, AnchorSerialize, AnchorDeserialize, InitSpace)]
pub struct PeerEntry {
    pub eid: u32,
    pub peer: [u8; 32],
}

impl PeerRegistry {
    pub const SEED: &'static [u8] = b"peers";

    pub fn upsert(&mut self, eid: u32, peer: [u8; 32]) -> Result<()> {
        if peer == [0u8; 32] {
            if let Some(i) = self.entries.iter().position(|e| e.eid == eid) {
                self.entries.swap_remove(i);
            }
            return Ok(());
        }
        if let Some(entry) = self.entries.iter_mut().find(|e| e.eid == eid) {
            entry.peer = peer;
            return Ok(());
        }
        require!(self.entries.len() < GRS_MAX_PEERS, OFTError::TooManyPeers);
        self.entries.push(PeerEntry { eid, peer });
        Ok(())
    }
}
