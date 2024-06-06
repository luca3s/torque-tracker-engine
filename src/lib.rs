pub mod file;
pub mod playback;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn load_test_1() {
        dbg!(
            "{:?}",
            file::file_handling::load_file(Path::new("test-1.it")).unwrap()
        );
        // file_formats::file_handling::load_file(Path::new("test-1.it")).unwrap();
    }
}
