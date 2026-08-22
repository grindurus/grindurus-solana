use crate::*;
use anchor_lang::solana_program::keccak;

const SEND_TO_OFFSET: usize = 0;
const SEND_AMOUNT_SD_OFFSET: usize = 32;
const COMPOSE_MSG_OFFSET: usize = 40;

/// Packed LZ sale payload: keccak256("GRS.sale") || id || asset || assetAmount || grsAmountSD || recipient.
/// `grsAmount` on the wire is OFT shared decimals; encode/decode convert to Solana local (9).
pub const SALE_MSG_LEN: usize = 192;
/// Packed LZ grant-vest payload: keccak256("GRS.grant") || to || amountSD || start || cliff || duration || bucket.
pub const GRANT_MSG_LEN: usize = 224;

pub fn sale_msg_type() -> [u8; 32] {
    keccak::hash(b"GRS.sale").to_bytes()
}

pub fn grant_msg_type() -> [u8; 32] {
    keccak::hash(b"GRS.grant").to_bytes()
}

fn u256_be_from_u64(v: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..].copy_from_slice(&v.to_be_bytes());
    out
}

fn u64_from_u256_be(word: &[u8]) -> Result<u64> {
    require!(word.len() == 32, OFTError::InvalidSaleMessage);
    require!(word[..24].iter().all(|b| *b == 0), OFTError::InvalidSaleMessage);
    let mut b = [0u8; 8];
    b.copy_from_slice(&word[24..32]);
    Ok(u64::from_be_bytes(b))
}

pub fn is_sale(message: &[u8]) -> bool {
    message.len() == SALE_MSG_LEN && message[0..32] == sale_msg_type()
}

pub fn is_grant(message: &[u8]) -> bool {
    message.len() == GRANT_MSG_LEN && message[0..32] == grant_msg_type()
}

pub fn encode_sale(id: u64, asset: Pubkey, asset_amount: u64, grs_amount: u64, recipient: Pubkey) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(SALE_MSG_LEN);
    encoded.extend_from_slice(&sale_msg_type());
    encoded.extend_from_slice(&u256_be_from_u64(id));
    encoded.extend_from_slice(asset.as_ref());
    encoded.extend_from_slice(&u256_be_from_u64(asset_amount));
    encoded.extend_from_slice(&u256_be_from_u64(grs_amount / GRS_LD2SD_RATE));
    encoded.extend_from_slice(recipient.as_ref());
    encoded
}

pub fn encode_grant(
    to: Pubkey,
    amount_ld: u64,
    start: u64,
    cliff_seconds: u64,
    duration_seconds: u64,
    bucket: u8,
) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(GRANT_MSG_LEN);
    encoded.extend_from_slice(&grant_msg_type());
    encoded.extend_from_slice(to.as_ref());
    encoded.extend_from_slice(&u256_be_from_u64(amount_ld / GRS_LD2SD_RATE));
    encoded.extend_from_slice(&u256_be_from_u64(start));
    encoded.extend_from_slice(&u256_be_from_u64(cliff_seconds));
    encoded.extend_from_slice(&u256_be_from_u64(duration_seconds));
    encoded.extend_from_slice(&u256_be_from_u64(bucket as u64));
    encoded
}

pub fn decode_sale(message: &[u8]) -> Result<(u64, Pubkey, u64, u64, Pubkey)> {
    require!(is_sale(message), OFTError::InvalidSaleMessage);
    let id = u64_from_u256_be(&message[32..64])?;
    // id == 0 is the OFT-compose forgery append path; home always publishes id ≥ 1.
    require!(id != 0, OFTError::InvalidSaleMessage);
    let mut asset_bytes = [0u8; 32];
    asset_bytes.copy_from_slice(&message[64..96]);
    let asset = Pubkey::from(asset_bytes);
    let asset_amount = u64_from_u256_be(&message[96..128])?;
    let grs_sd = u64_from_u256_be(&message[128..160])?;
    let grs_amount = grs_sd
        .checked_mul(GRS_LD2SD_RATE)
        .ok_or(error!(OFTError::InvalidSaleMessage))?;
    let mut recipient_bytes = [0u8; 32];
    recipient_bytes.copy_from_slice(&message[160..192]);
    let recipient = Pubkey::from(recipient_bytes);
    Ok((id, asset, asset_amount, grs_amount, recipient))
}

pub fn decode_grant(message: &[u8]) -> Result<(Pubkey, u64, u64, u64, u64, u8)> {
    require!(is_grant(message), OFTError::InvalidSaleMessage);
    let mut to_bytes = [0u8; 32];
    to_bytes.copy_from_slice(&message[32..64]);
    let to = Pubkey::from(to_bytes);
    let amount_sd = u64_from_u256_be(&message[64..96])?;
    let amount_ld = amount_sd
        .checked_mul(GRS_LD2SD_RATE)
        .ok_or(error!(OFTError::InvalidSaleMessage))?;
    let start = u64_from_u256_be(&message[96..128])?;
    let cliff_seconds = u64_from_u256_be(&message[128..160])?;
    let duration_seconds = u64_from_u256_be(&message[160..192])?;
    let bucket = u64_from_u256_be(&message[192..224])?;
    require!(bucket <= 11, OFTError::InvalidSaleMessage);
    Ok((to, amount_ld, start, cliff_seconds, duration_seconds, bucket as u8))
}

pub fn encode(
    send_to: [u8; 32],
    amount_sd: u64,
    sender: Pubkey,
    compose_msg: &Option<Vec<u8>>,
) -> Vec<u8> {
    if let Some(msg) = compose_msg {
        let mut encoded = Vec::with_capacity(72 + msg.len()); // 32 + 8 + 32
        encoded.extend_from_slice(&send_to);
        encoded.extend_from_slice(&amount_sd.to_be_bytes());
        encoded.extend_from_slice(sender.to_bytes().as_ref());
        encoded.extend_from_slice(&msg);
        encoded
    } else {
        let mut encoded = Vec::with_capacity(40); // 32 + 8
        encoded.extend_from_slice(&send_to);
        encoded.extend_from_slice(&amount_sd.to_be_bytes());
        encoded
    }
}

pub fn send_to(message: &[u8]) -> [u8; 32] {
    let mut send_to = [0; 32];
    send_to.copy_from_slice(&message[SEND_TO_OFFSET..SEND_AMOUNT_SD_OFFSET]);
    send_to
}

pub fn amount_sd(message: &[u8]) -> u64 {
    let mut amount_sd_bytes = [0; 8];
    amount_sd_bytes.copy_from_slice(&message[SEND_AMOUNT_SD_OFFSET..COMPOSE_MSG_OFFSET]);
    u64::from_be_bytes(amount_sd_bytes)
}

pub fn compose_msg(message: &[u8]) -> Option<Vec<u8>> {
    if is_sale(message) || is_grant(message) {
        return None;
    }
    if message.len() > COMPOSE_MSG_OFFSET {
        Some(message[COMPOSE_MSG_OFFSET..].to_vec())
    } else {
        None
    }
}

