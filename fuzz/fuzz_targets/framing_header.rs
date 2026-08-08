#![no_main]
use common::framing::decode_destination;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = decode_destination(data);
});
