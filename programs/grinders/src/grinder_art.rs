/// Metaplex metadata `uri` (max 200 bytes). SVG is rendered off-chain at this URL.
pub fn token_json_uri(custodian_id: u64) -> String {
    format!("https://grindurus.xyz/solana/custodian/{custodian_id}")
}
