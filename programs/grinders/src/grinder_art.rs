use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak::hashv;

/// EVM `abi.encodePacked` uint256: 32-byte big-endian, zero-padded.
fn u256_be(v: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&v.to_be_bytes());
    out
}

/// Matches `uint256(keccak256(...))` in GrinderArt.sol (trait shifts use the low bits).
fn seed_from_hash(hash: &[u8; 32]) -> u64 {
    u64::from_be_bytes(hash[24..32].try_into().unwrap())
}

const MASK: [u8; 144] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x68, 0x00, 0xaa, 0xa4, 0x00,
    0x01, 0xa8, 0x02, 0xaa, 0x90, 0x00, 0x06, 0xac, 0x02, 0xaa, 0xaa, 0x90, 0x1a, 0xa0, 0x02, 0xaa,
    0xa5, 0xaa, 0x1a, 0x80, 0x01, 0xaa, 0x5e, 0xa6, 0xaa, 0x80, 0x00, 0x05, 0x56, 0x2e, 0x95, 0xaa,
    0x00, 0x09, 0x57, 0x95, 0x9a, 0x00, 0x00, 0x25, 0x55, 0x56, 0x80, 0x00, 0x08, 0x15, 0x55, 0x5b,
    0xae, 0x00, 0x28, 0x57, 0x75, 0x56, 0x08, 0x20, 0x28, 0x5d, 0x75, 0x56, 0x08, 0xa0, 0x9c, 0x7d,
    0x7f, 0x54, 0xaa, 0xb0, 0x8c, 0x75, 0x55, 0x50, 0x00, 0x00, 0x8c, 0x76, 0x95, 0x60, 0x00, 0x00,
    0x8c, 0x36, 0x95, 0x40, 0x00, 0x00, 0x43, 0x36, 0xa5, 0x00, 0x00, 0x00, 0x03, 0x75, 0xa5, 0x00,
    0x00, 0x00, 0x00, 0xd5, 0xa9, 0x6a, 0xaa, 0x80, 0x00, 0x35, 0x6a, 0x6a, 0xa8, 0x00, 0x00, 0x0d,
    0x6a, 0x5a, 0xa0, 0x00, 0x00, 0x00, 0x5a, 0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa0, 0x00, 0x00,
];

const BG: [u8; 24] = [
    0x00, 0x00, 0x00, 0x0a, 0x06, 0x12, 0x1a, 0x0a, 0x22, 0x14, 0x08, 0x28, 0x0e, 0x10, 0x20, 0x0c,
    0x18, 0x30, 0x18, 0x40, 0x20, 0x20, 0x18, 0x50,
];
const BODY: [u8; 24] = [
    0x1c, 0x24, 0x44, 0x1a, 0x20, 0x38, 0x1e, 0x28, 0x50, 0x1c, 0x2a, 0x48, 0x14, 0x22, 0x38, 0x10,
    0x18, 0x30, 0x14, 0x1c, 0x40, 0x18, 0x20, 0x38,
];
const HI: [u8; 24] = [
    0xff, 0xff, 0xff, 0xf0, 0xf4, 0xff, 0xf5, 0xe6, 0xc8, 0xe8, 0xf0, 0xff, 0xe0, 0xe8, 0xf0, 0xff,
    0xf8, 0xe0, 0xf0, 0xff, 0xf0, 0xff, 0xf0, 0xe8,
];
const ACC: [u8; 24] = [
    0xff, 0x2d, 0x8c, 0xff, 0x4d, 0x8d, 0xff, 0x1a, 0x6e, 0xff, 0x6a, 0xb0, 0xff, 0x3d, 0x00, 0xff,
    0xd4, 0x00, 0x2d, 0xff, 0x9a, 0x00, 0xe5, 0xff,
];
const HORN: [u8; 48] = [
    0xf5, 0xe6, 0xc8, 0xff, 0xd7, 0x00, 0xe8, 0xdc, 0xc8, 0xc0, 0xc0, 0xc8, 0xff, 0xb6, 0xc1, 0xb8,
    0x73, 0x33, 0xff, 0xf8, 0xff, 0xff, 0x2a, 0x4a, 0xd4, 0xaf, 0x37, 0xe6, 0xc3, 0x5c, 0xff, 0x8c,
    0x42, 0xa8, 0xe6, 0xcf, 0xf0, 0xe6, 0x8c, 0xe0, 0xb0, 0xff, 0x98, 0xd8, 0xc8, 0xff, 0xe4, 0xc4,
];

fn rgb(table: &[u8], i: usize) -> u32 {
    let i = i * 3;
    ((table[i] as u32) << 16) | ((table[i + 1] as u32) << 8) | (table[i + 2] as u32)
}

fn hex_rgb(color: u32) -> String {
    format!("#{:06x}", color & 0xffffff)
}

fn u_str(v: usize) -> String {
    if v < 10 {
        return v.to_string();
    }
    format!("{v}")
}

fn rect(x: usize, y: usize, w: usize, h: usize, color: u32) -> String {
    format!(
        "<rect x='{}' y='{}' width='{}' height='{}' fill='{}'/>",
        u_str(x),
        u_str(y),
        u_str(w),
        u_str(h),
        hex_rgb(color)
    )
}

fn svg(seed: u64) -> String {
    let bg = rgb(&BG, ((seed >> 4) % 8) as usize);
    let body = rgb(&BODY, ((seed >> 8) % 8) as usize);
    let hi = rgb(&HI, ((seed >> 12) % 8) as usize);
    let acc = rgb(&ACC, ((seed >> 16) % 8) as usize);
    let horn = rgb(&HORN, ((seed >> 20) % 16) as usize);

    let mut out = String::from(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' shape-rendering='crispEdges'>",
    );
    out.push_str(&rect(0, 0, 24, 24, bg));

    let mut bit = 0usize;
    for y in 0..24 {
        let mut x = 0usize;
        while x < 24 {
            let byte_index = bit >> 2;
            let shift = 6 - ((bit & 3) << 1);
            let c = (MASK[byte_index] >> shift) & 3;
            bit += 1;
            if c == 0 {
                x += 1;
                continue;
            }
            let x0 = x;
            x += 1;
            while x < 24 {
                let bi2 = bit >> 2;
                let sh2 = 6 - ((bit & 3) << 1);
                let c2 = (MASK[bi2] >> sh2) & 3;
                if c2 != c {
                    break;
                }
                bit += 1;
                x += 1;
            }
            let color = if c == 1 {
                body
            } else if c == 3 {
                acc
            } else if y <= 5 {
                horn
            } else {
                hi
            };
            out.push_str(&rect(x0, y, x - x0, 1, color));
        }
    }
    out.push_str("</svg>");
    out
}

fn pubkey_hex(pk: &Pubkey) -> String {
    let bytes = pk.to_bytes();
    let mut out = String::with_capacity(66);
    out.push_str("0x");
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn bytes32_hex(kind: &[u8; 32]) -> String {
    let mut out = String::with_capacity(66);
    out.push_str("0x");
    for b in kind {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// `keccak256(abi.encodePacked("grindurus.grinder", chainId, tokenId, kind))`.
/// Solana has no `block.chainid`; use `0` (EVM uses the deployment chain id).
fn art_seed(custodian_id: u64, custodian_kind: &[u8; 32]) -> u64 {
    const CHAIN_ID: u64 = 0;
    let hash = hashv(&[
        b"grindurus.grinder",
        &u256_be(CHAIN_ID),
        &u256_be(custodian_id),
        custodian_kind,
    ]);
    seed_from_hash(&hash.0)
}

pub fn token_json_uri(
    custodian_id: u64,
    _custodian_wallet: &Pubkey,
    _custodian_kind: &[u8; 32],
) -> String {
    // Metaplex `uri` must stay small: full SVG JSON exceeds the BPF heap during `mint`.
    format!("https://grindurus.xyz/solana/custodian/{custodian_id}")
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() { input[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 63) as usize] as char);
        out.push(TABLE[((triple >> 12) & 63) as usize] as char);
        if i + 1 < input.len() {
            out.push(TABLE[((triple >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < input.len() {
            out.push(TABLE[(triple & 63) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}
