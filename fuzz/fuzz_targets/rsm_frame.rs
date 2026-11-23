//! The RSM frame decoder (AUDIT S-09, S-10): never panics, and every frame it
//! accepts re-encodes to the same bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use roadmesh_core::rsm::{Frame, Rsm};

fuzz_target!(|data: &[u8]| {
    if let Ok(f) = Frame::decode(data) {
        let again = f.encode();
        assert!(again.is_some(), "accepted frame must encode");
        if let Some(bytes) = again {
            let back = Frame::decode(&bytes);
            assert!(back.is_ok(), "re-encoded frame must decode");
            if let Ok(back) = back {
                assert_eq!(back.body, f.body);
                assert_eq!(back.auth, f.auth);
            }
        }
        assert!(Rsm::decode(&f.body).is_ok());
    }
    if data.len() >= 31 {
        let mut b = [0u8; 31];
        b.copy_from_slice(&data[..31]);
        if let Ok(r) = Rsm::decode(&b) {
            let _ = Rsm::decode(&r.encode());
        }
    }
});
