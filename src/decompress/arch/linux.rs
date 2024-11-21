pub const EMBEDDED_7Z: &[u8] = include_bytes!("../../../assets/7zz");
pub const SEVENZZ: &str = "7zz";

pub fn decode_7z_output(input: &[u8]) -> String {
    String::from_utf8_lossy(input).to_string()
}

pub fn set_creation_flags(_cmd: &mut Command) {}
