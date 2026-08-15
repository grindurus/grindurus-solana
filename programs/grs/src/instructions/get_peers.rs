use crate::*;

#[derive(Accounts)]
pub struct GetPeers<'info> {
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        seeds = [PeerRegistry::SEED, oft_store.key().as_ref()],
        bump = peer_registry.bump,
        has_one = oft_store
    )]
    pub peer_registry: Account<'info, PeerRegistry>,
}

impl GetPeers<'_> {
    pub fn apply(ctx: &Context<GetPeers>) -> Result<Vec<PeerEntry>> {
        Ok(ctx.accounts.peer_registry.entries.clone())
    }
}
