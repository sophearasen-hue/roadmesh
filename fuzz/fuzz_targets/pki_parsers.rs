//! Certificate, auth envelope, revocation list and misbehaviour report parsers,
//! on the phone and on the server (AUDIT S-10).
#![no_main]

use std::sync::{Mutex, OnceLock};

use libfuzzer_sys::fuzz_target;
use roadmesh_core::pki::{AuthEnvelope, Certificate, MisbehaviourReport, RevocationList};

#[path = "common.rs"]
mod common;

static FIX: OnceLock<Mutex<common::Fixture>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let fix = FIX.get_or_init(|| Mutex::new(common::Fixture::new()));
    let Ok(mut fix) = fix.lock() else {
        return;
    };
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let input = match mode % 3 {
        0 => rest.to_vec(),
        1 => common::mutate(&fix.report, rest),
        _ => common::mutate(&fix.crl, rest),
    };
    if let Ok(c) = Certificate::decode(&input) {
        assert_eq!(Certificate::decode(&c.encode()), Ok(c));
    }
    if let Ok(e) = AuthEnvelope::decode(&input) {
        assert_eq!(AuthEnvelope::decode(&e.encode()), Ok(e));
    }
    let _ = RevocationList::decode(&input, 1 << 18);
    let mut v = fix.verifier();
    let _ = v.load_revocation_list(&input);
    if let Ok(r) = MisbehaviourReport::decode(&input, 16) {
        let _ = r.to_be_signed();
    }
    // the server's report handler on untrusted bytes
    let _ = fix.ca.report(&input, common::NOW as u32);
});
