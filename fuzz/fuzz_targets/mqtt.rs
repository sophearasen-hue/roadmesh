//! MQTT codec (Step 9): never panics; whatever decodes re-encodes to the same bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use roadmesh_core::mqtt::{MAX_PACKET, decode, encode};

fuzz_target!(|data: &[u8]| {
    let mut at = 0;
    while let Ok(Some((p, used))) = decode(&data[at..]) {
        assert!(used > 0 && used <= MAX_PACKET);
        let again = encode(&p);
        assert_eq!(decode(&again), Ok(Some((p, again.len()))));
        at += used;
    }
});
