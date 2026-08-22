#[cfg(test)]
mod test_msg_codec {
    use anchor_lang::prelude::Pubkey;
    use grs::compose_msg_codec;
    use grs::msg_codec;

    #[test]
    fn test_msg_codec_with_compose_msg() {
        let send_to: [u8; 32] = [1; 32];
        let amount_sd: u64 = 123456789;
        let sender: Pubkey = Pubkey::new_unique();
        let compose_msg: Option<Vec<u8>> = Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 0]);
        let encoded = msg_codec::encode(send_to, amount_sd, sender, &compose_msg);
        assert_eq!(encoded.len(), 72 + compose_msg.clone().unwrap().len());
        assert_eq!(msg_codec::send_to(&encoded), send_to);
        assert_eq!(msg_codec::amount_sd(&encoded), amount_sd);
        assert_eq!(
            msg_codec::compose_msg(&encoded),
            Some([sender.to_bytes().as_ref(), compose_msg.unwrap().as_slice()].concat())
        );
    }

    #[test]
    fn test_msg_codec_without_compose_msg() {
        let send_to: [u8; 32] = [1; 32];
        let amount_sd: u64 = 123456789;
        let sender: Pubkey = Pubkey::new_unique();
        let compose_msg: Option<Vec<u8>> = None;
        let encoded = msg_codec::encode(send_to, amount_sd, sender, &compose_msg);
        assert_eq!(encoded.len(), 40);
        assert_eq!(msg_codec::send_to(&encoded), send_to);
        assert_eq!(msg_codec::amount_sd(&encoded), amount_sd);
        assert_eq!(msg_codec::compose_msg(&encoded), None);
    }

    #[test]
    fn test_compose_msg_codec() {
        let nonce: u64 = 123456789;
        let src_eid: u32 = 987654321;
        let amount_ld: u64 = 123456789;
        let compose_from: [u8; 32] = [1; 32];
        let compose_msg: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 0];
        let encoded = compose_msg_codec::encode(
            nonce,
            src_eid,
            amount_ld,
            &[&compose_from[..], &compose_msg].concat(),
        );
        assert_eq!(encoded.len(), 20 + [&compose_from[..], &compose_msg].concat().len());
        assert_eq!(compose_msg_codec::nonce(&encoded), nonce);
        assert_eq!(compose_msg_codec::src_eid(&encoded), src_eid);
        assert_eq!(compose_msg_codec::amount_ld(&encoded), amount_ld);
        assert_eq!(compose_msg_codec::compose_msg(&encoded), compose_msg);
    }

    #[test]
    fn test_sale_msg_roundtrip_matches_evm_layout() {
        let asset = Pubkey::new_unique();
        let recipient = Pubkey::new_unique();
        let encoded = msg_codec::encode_sale(3, asset, 10_000_000, 1_000_000_000, recipient);
        assert_eq!(encoded.len(), 192);
        assert!(msg_codec::is_sale(&encoded));
        assert_eq!(msg_codec::compose_msg(&encoded), None);
        let (id, q, asset_amount, grs_amount, r) = msg_codec::decode_sale(&encoded).unwrap();
        assert_eq!(id, 3);
        assert_eq!(q, asset);
        assert_eq!(asset_amount, 10_000_000);
        assert_eq!(grs_amount, 1_000_000_000);
        assert_eq!(r, recipient);
        let expected_magic: [u8; 32] = [
            0x9a, 0xd0, 0x4f, 0x5e, 0x83, 0x38, 0x4a, 0x6d, 0x14, 0x3d, 0xcd, 0xba, 0xb3, 0xe4,
            0x98, 0xbd, 0xe0, 0x25, 0x7b, 0xcd, 0xd7, 0x78, 0x06, 0x84, 0x3b, 0x0c, 0x11, 0x5c,
            0xbe, 0xf6, 0x5b, 0xe9,
        ];
        assert_eq!(&encoded[0..32], &expected_magic);
        let mut sd = [0u8; 8];
        sd.copy_from_slice(&encoded[152..160]);
        assert_eq!(u64::from_be_bytes(sd), 1_000_000);
    }

    #[test]
    fn test_grant_msg_roundtrip() {
        let to = Pubkey::new_unique();
        let encoded = msg_codec::encode_grant(to, 1_000_000_000, 1_700_000_000, 86_400, 259_200, 4);
        assert_eq!(encoded.len(), 224);
        assert!(msg_codec::is_grant(&encoded));
        assert!(!msg_codec::is_sale(&encoded));
        assert_eq!(msg_codec::compose_msg(&encoded), None);
        let (decoded_to, amount, start, cliff, duration, bucket) =
            msg_codec::decode_grant(&encoded).unwrap();
        assert_eq!(decoded_to, to);
        assert_eq!(amount, 1_000_000_000);
        assert_eq!(start, 1_700_000_000);
        assert_eq!(cliff, 86_400);
        assert_eq!(duration, 259_200);
        assert_eq!(bucket, 4);
    }

    #[test]
    fn test_oft_credit_is_not_sale_and_sd_matches_solana_ld() {
        let send_to: [u8; 32] = [0x51; 32];
        let amount_sd: u64 = 5_000_000; // 5 GRS at 6 shared decimals
        let encoded = msg_codec::encode(send_to, amount_sd, Pubkey::default(), &None);
        assert_eq!(encoded.len(), 40);
        assert!(!msg_codec::is_sale(&encoded));
        assert_eq!(msg_codec::send_to(&encoded), send_to);
        assert_eq!(msg_codec::amount_sd(&encoded), amount_sd);
        assert_eq!(amount_sd * grs::GRS_LD2SD_RATE, 5 * grs::GRS_ONE_LD);
    }
}
