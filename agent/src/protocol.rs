pub const WINDOW_CLASS: &str = "CorralAgent";
pub const HOOK_PROC: &[u8] = b"corral_hook_proc\0";

pub const CMD_SET_HIDDEN: usize = 1;
pub const CMD_DETACH: usize = 2;
pub const CMD_SET_OWNER: usize = 3;

pub fn encode_names<S: AsRef<str>>(names: &[S]) -> Vec<u16> {
    let mut out = Vec::new();
    for name in names {
        out.extend(name.as_ref().encode_utf16());
        out.push(0);
    }
    out
}

pub fn decode_names(data: &[u16]) -> Vec<String> {
    data.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        let names = ["феникс.docx", "a b.txt"];
        assert_eq!(decode_names(&encode_names(&names)), names);
        assert!(decode_names(&encode_names::<&str>(&[])).is_empty());
    }
}
