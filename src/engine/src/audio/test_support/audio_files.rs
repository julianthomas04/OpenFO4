use std::path::Path;

/// Writes `bytes` as the file `file_key` below the audio folder of the install at `root`.
pub fn write_audio_file(root: &Path, file_key: &str, bytes: &[u8]) {
    let path = root.join("audio").join(file_key);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
