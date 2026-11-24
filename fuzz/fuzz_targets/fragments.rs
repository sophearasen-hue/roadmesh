//! GATT fragment reassembly (Step 6): never panics, never yields more than a frame.
#![no_main]

use libfuzzer_sys::fuzz_target;
use roadmesh_core::fragment::{MAX_FRAME, Reassembler, split};

fuzz_target!(|data: &[u8]| {
    let mut r = Reassembler::new();
    for packet in data.split(|b| *b == 0xff) {
        if let Some(f) = r.push(packet) {
            assert!(f.len() <= MAX_FRAME);
        }
    }
    if let Some(&size) = data.first() {
        let frame = &data[1..data.len().min(1 + MAX_FRAME)];
        if let Some(parts) = split(frame, 3, usize::from(size)) {
            let mut r = Reassembler::new();
            let out = parts.iter().filter_map(|p| r.push(p)).last();
            assert_eq!(out.as_deref(), Some(frame));
        }
    }
});
