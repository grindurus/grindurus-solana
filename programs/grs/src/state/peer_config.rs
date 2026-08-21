use crate::*;

pub const ENFORCED_OPTIONS_SEND_MAX_LEN: usize = 512;
pub const ENFORCED_OPTIONS_SEND_AND_CALL_MAX_LEN: usize = 1024;

#[account]
#[derive(InitSpace)]
pub struct PeerConfig {
    pub peer_address: [u8; 32],
    pub enforced_options: EnforcedOptions,
    pub outbound_rate_limiter: Option<RateLimiter>,
    pub inbound_rate_limiter: Option<RateLimiter>,
    pub fee_bps: Option<u16>,
    pub bump: u8,
}

#[derive(Clone, Default, AnchorSerialize, AnchorDeserialize, InitSpace)]
pub struct RateLimiter {
    pub capacity: u64,
    pub tokens: u64,
    pub refill_per_second: u64,
    pub last_refill_time: u64,
}

impl RateLimiter {
    pub fn set_rate(&mut self, refill_per_second: u64) -> Result<()> {
        self.refill(0)?;
        self.refill_per_second = refill_per_second;
        Ok(())
    }

    pub fn set_capacity(&mut self, capacity: u64) -> Result<()> {
        self.capacity = capacity;
        self.tokens = capacity;
        self.last_refill_time = Clock::get()?.unix_timestamp.try_into().unwrap();
        Ok(())
    }

    pub fn refill(&mut self, extra_tokens: u64) -> Result<()> {
        let mut new_tokens = extra_tokens;
        let current_time: u64 = Clock::get()?.unix_timestamp.try_into().unwrap();
        if current_time > self.last_refill_time {
            let time_elapsed_in_seconds = current_time - self.last_refill_time;
            new_tokens = new_tokens
                .saturating_add(time_elapsed_in_seconds.saturating_mul(self.refill_per_second));
        }
        self.tokens = std::cmp::min(self.capacity, self.tokens.saturating_add(new_tokens));

        self.last_refill_time = current_time;
        Ok(())
    }

    pub fn try_consume(&mut self, amount: u64) -> Result<()> {
        self.refill(0)?;
        match self.tokens.checked_sub(amount) {
            Some(new_tokens) => {
                self.tokens = new_tokens;
                Ok(())
            },
            None => Err(error!(OFTError::RateLimitExceeded)),
        }
    }
}

#[derive(Clone, Default, AnchorSerialize, AnchorDeserialize, InitSpace)]
pub struct EnforcedOptions {
    #[max_len(ENFORCED_OPTIONS_SEND_MAX_LEN)]
    pub send: Vec<u8>,
    #[max_len(ENFORCED_OPTIONS_SEND_AND_CALL_MAX_LEN)]
    pub send_and_call: Vec<u8>,
}

impl EnforcedOptions {
    pub fn get_enforced_options(&self, composed_msg: &Option<Vec<u8>>) -> Vec<u8> {
        if composed_msg.is_none() {
            self.send.clone()
        } else {
            self.send_and_call.clone()
        }
    }

    pub fn combine_options(
        &self,
        compose_msg: &Option<Vec<u8>>,
        extra_options: &Vec<u8>,
    ) -> Result<Vec<u8>> {
        let enforced_options = self.get_enforced_options(compose_msg);
        oapp::options::combine_options(enforced_options, extra_options)
    }

    /// Apply the same Type-3 executor lzReceive options to SEND and SEND_AND_CALL.
    pub fn set_lz_receive_budget(&mut self, gas: u128, value: u128) {
        let opts = encode_executor_lz_receive_option(gas, value);
        self.send = opts.clone();
        self.send_and_call = opts;
    }
}

/// LayerZero Type-3 options: executor lzReceive (matches EVM `OptionsBuilder.addExecutorLzReceiveOption`).
pub fn encode_executor_lz_receive_option(gas: u128, value: u128) -> Vec<u8> {
    const TYPE_3: u16 = 3;
    const EXECUTOR_WORKER_ID: u8 = 1;
    const OPTION_TYPE_LZRECEIVE: u8 = 1;

    let mut option_payload = Vec::with_capacity(32);
    option_payload.extend_from_slice(&gas.to_be_bytes());
    if value != 0 {
        option_payload.extend_from_slice(&value.to_be_bytes());
    }

    let option_size = (option_payload.len() + 1) as u16; // +1 for option type
    let mut out = Vec::with_capacity(2 + 1 + 2 + 1 + option_payload.len());
    out.extend_from_slice(&TYPE_3.to_be_bytes());
    out.push(EXECUTOR_WORKER_ID);
    out.extend_from_slice(&option_size.to_be_bytes());
    out.push(OPTION_TYPE_LZRECEIVE);
    out.extend_from_slice(&option_payload);
    out
}

utils::generate_account_size_test!(EnforcedOptions, enforced_options_test);
