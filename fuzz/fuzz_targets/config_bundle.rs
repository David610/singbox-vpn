#![no_main]
use config::SignedBundle;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _: Result<SignedBundle, _> = serde_json::from_slice(data);
});
