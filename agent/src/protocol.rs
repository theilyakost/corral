pub const WINDOW_CLASS: &str = "CorralAgent";
pub const HOOK_PROC: &[u8] = b"corral_hook_proc\0";

pub const CMD_SET_HIDDEN: usize = 1;
pub const CMD_DETACH: usize = 2;
pub const CMD_SET_OWNER: usize = 3;
pub const CMD_DRAG_BEGIN: usize = 4;
pub const CMD_DRAG_END: usize = 5;
pub const CMD_CONTEXT_MENU: usize = 6;

pub const MAIN_CLASS: &str = "CorralMain";
pub const EVT_RENAME: usize = 100;
pub const EVT_MENU_DONE: usize = 101;

pub fn encode_point(x: i32, y: i32) -> [u16; 4] {
    [x as u32 as u16, (x as u32 >> 16) as u16, y as u32 as u16, (y as u32 >> 16) as u16]
}

pub fn decode_point(words: &[u16]) -> Option<(i32, i32)> {
    let [a, b, c, d] = words.get(..4)?.try_into().ok()?;
    Some(((a as u32 | (b as u32) << 16) as i32, (c as u32 | (d as u32) << 16) as i32))
}

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
    fn points_roundtrip() {
        for (x, y) in [(0, 0), (-1920, 1032), (70000, -5)] {
            assert_eq!(decode_point(&encode_point(x, y)), Some((x, y)));
        }
        assert_eq!(decode_point(&[1, 2]), None);
    }

    #[test]
    fn names_roundtrip() {
        let names = ["феникс.docx", "a b.txt"];
        assert_eq!(decode_names(&encode_names(&names)), names);
        assert!(decode_names(&encode_names::<&str>(&[])).is_empty());
    }
}
