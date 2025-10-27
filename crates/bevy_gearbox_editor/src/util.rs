pub(crate) fn canonicalize_entity_u64(raw: u64) -> u64 {
    let low = (raw & 0xFFFF_FFFF) as u32;
    let high = ((raw >> 32) & 0xFFFF_FFFF) as u32;
    // If it already looks like to_bits encoding (high != 0 or low in NonZero niche range), keep it
    // Otherwise, treat as plain index with generation 0 and convert to to_bits (low = !index)
    let looks_like_bits = high != 0 || low > 0x7FFF_FFFF;
    if looks_like_bits { raw } else { ((!low) as u64) }
}

pub(crate) fn parse_entity_str_to_bits(s: &str) -> Option<u64> {
    // Prefer explicit indexvgeneration pattern
    if let Some(vpos) = s.find('v') {
        let (lhs, rhs) = s.split_at(vpos);
        let idx_txt: String = lhs.chars().filter(|c| c.is_ascii_digit()).collect();
        let gen_txt: String = rhs[1..].chars().filter(|c| c.is_ascii_digit()).collect();
        if !idx_txt.is_empty() && !gen_txt.is_empty() {
            if let (Ok(index), Ok(gen)) = (idx_txt.parse::<u32>(), gen_txt.parse::<u32>()) {
                let low = !index as u32 as u64;
                let high = (gen as u64) << 32;
                return Some(high | low);
            }
        }
    }
    // Fallback: first contiguous run of digits as a number
    let mut digits = String::new();
    let mut in_run = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() { digits.push(ch); in_run = true; } else if in_run { break; }
    }
    if !digits.is_empty() { if let Ok(n) = digits.parse::<u64>() { return Some(canonicalize_entity_u64(n)); } }
    None
}


