#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    impulse_engine::file::file_handling::load_slice(data);
});
