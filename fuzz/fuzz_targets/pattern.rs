#![no_main]

use libfuzzer_sys::fuzz_target;
extern crate impulse_engine;

fuzz_target!(|data: &[u8]| {
    impulse_engine::file::impulse_format::pattern::load_pattern(data);
});
