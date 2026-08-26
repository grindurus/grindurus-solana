use anchor_lang::prelude::*;

use crate::state::GrindersState;
use crate::HeartbeatEvent;

pub fn heartbeat(grinders: &mut Account<GrindersState>) -> Result<()> {
    grinders.heartbeat_at = Clock::get()?.unix_timestamp;
    emit!(HeartbeatEvent {
        heartbeat_at: grinders.heartbeat_at,
    });
    Ok(())
}
