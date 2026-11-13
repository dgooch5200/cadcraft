//! The product code as a device card shows it: contracted to fit the body, never stored.

use super::Product;

/// Characters a card body can show before it would cut a word.
pub const CANVAS_CODE_MAX: usize = 10;

/// Product families, longest match first: the name's prefix and what it contracts to.
const FAMILIES: [(&str, &str); 12] = [
    ("rogue outcast", "ROC"),
    ("maverick storm", "MS"),
    ("maverick force", "MF"),
    ("maverick silens", "MSI"),
    ("colorado solo", "CS"),
    ("colorado pxl", "CPXL"),
    ("colorado", "C"),
    ("maverick", "M"),
    ("rogue", "R"),
    ("ovation", "OV"),
    ("colordash", "CD"),
    ("riglink", "RL"),
];

/// Word pairs contracted before single words: BEAM then WASH is BW.
const PHRASES: [(&str, &str, &str); 2] = [("BEAM", "WASH", "BW"), ("SOLO", "WASH", "SW")];

const WORDS: [(&str, &str); 18] = [
    ("PROFILE", "P"),
    ("SPOT", "S"),
    ("BEAM", "B"),
    ("WASH", "W"),
    ("HYBRID", "HY"),
    ("ZOOM", "Z"),
    ("QUAD", "Q"),
    ("SOLO", "SO"),
    ("CURVE", "CRV"),
    ("BATTEN", "BAT"),
    ("PANEL", "PNL"),
    ("STADIUM", "STAD"),
    ("CONNECT", "CON"),
    ("GATEWAY", "GW"),
    ("CONTACT", "CNT"),
    ("STORE", "STR"),
    ("SPLITTER", "SPL"),
    ("SWITCH", "SW"),
];

/// Hand-chosen codes, keyed by the product's real code, for names the rules cannot guess.
const OVERRIDES: [(&str, &str); 6] = [
    ("EPIX FLEX DRIVE", "EpixFLXDRV"),
    ("LRC-72-X1-RGBA-12-5M-WA", "LRC4IN1RWP"),
    ("CA-PCONTOP-EDISON-3M", "PCT1-EDSN"),
    ("CA-PCONTOP-SCHUKO-3M", "PCT1-SCHKO"),
    ("CA-PCONTOP-BE-3M-US", "PCT1-BE-US"),
    ("CA-PCONTOP-BE-3M-INT", "PCT1-BEINT"),
];

fn deaccent(c: char) -> char {
    match c {
        'À'..='Å' => 'A',
        'È'..='Ë' => 'E',
        'Ì'..='Ï' => 'I',
        'Ò'..='Ö' => 'O',
        'Ù'..='Ü' => 'U',
        'à'..='å' => 'a',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ò'..='ö' => 'o',
        'ù'..='ü' => 'u',
        'Ç' => 'C',
        'ç' => 'c',
        'Ñ' => 'N',
        'ñ' => 'n',
        c => c,
    }
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || "—–_/-".contains(c)
}

/// Words of `rest`: split at separators and at a lower-to-upper step ("BeamWash"), upper-cased.
fn tokens(rest: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in rest.chars() {
        if is_separator(c) || (prev_lower && c.is_uppercase()) {
            words.push(std::mem::take(&mut cur));
        }
        if !is_separator(c) {
            cur.push(c);
        }
        prev_lower = c.is_lowercase();
    }
    words.push(cur);
    words
        .into_iter()
        .map(|w| w.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '+').collect::<String>().to_uppercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// `product.code` as the card body shows it, at most [`CANVAS_CODE_MAX`] characters. A code that
/// already fits is returned as is; otherwise the name is contracted (family prefix, designers'
/// short words, then trimming the longest words), always keeping digit tokens and tokens of two
/// characters or fewer, which are what tell variants apart. Schedules and exports keep the code.
pub fn canvas_code(product: &Product) -> String {
    if let Some((_, short)) = OVERRIDES.iter().find(|(code, _)| *code == product.code) {
        return (*short).into();
    }
    if product.code.chars().count() <= CANVAS_CODE_MAX {
        return product.code.clone();
    }
    // Library names are uncapped; the contraction loop below is quadratic in their length.
    let name: String = product.name.chars().take(200).map(deaccent).collect::<String>().trim().to_string();
    let lower = name.to_lowercase();
    let (prefix, rest) = FAMILIES
        .iter()
        .find(|(family, _)| lower.starts_with(family))
        .map_or(("", name.as_str()), |(family, short)| (*short, name.get(family.len()..).unwrap_or("")));
    let words = tokens(rest);
    let mut cut: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(a) = words.get(i) {
        if let Some((_, _, short)) = PHRASES.iter().find(|(x, y, _)| x == a && words.get(i + 1).is_some_and(|b| b == y)) {
            cut.push((*short).into());
            i += 2;
        } else {
            cut.push(WORDS.iter().find(|(w, _)| w == a).map_or(a.clone(), |(_, s)| (*s).into()));
            i += 1;
        }
    }
    let fixed: Vec<bool> = cut.iter().map(|t| t.len() <= 2 || t.chars().any(|c| c.is_ascii_digit())).collect();
    let total = |cut: &[String]| prefix.len() + cut.iter().map(String::len).sum::<usize>();
    while total(&cut) > CANVAS_CODE_MAX {
        let longest = cut
            .iter()
            .enumerate()
            .filter(|(i, t)| !fixed.get(*i).copied().unwrap_or(true) && t.len() > 1)
            .max_by_key(|(i, t)| (t.len(), usize::MAX - i))
            .map(|(i, _)| i);
        match longest.and_then(|at| cut.get_mut(at)) {
            Some(t) => {
                t.pop();
            }
            None => break,
        }
    }
    let built = format!("{prefix}{}", cut.concat());
    let source = if built.is_empty() { &product.code } else { &built };
    source.chars().take(CANVAS_CODE_MAX).collect()
}
