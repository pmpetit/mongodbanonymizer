//! Data masking functions – one implementation per category defined in
//! `data/identifier_category.csv`.
//!
//! # Categories
//! | Category               | Function                  |
//! |------------------------|---------------------------|
//! | PRESERVE_TOKEN         | [`preserve_token`]        |
//! | REDACT_ALPHANUMERIC    | [`redact_alphanumeric`]   |
//! | MASK_CONTACT_URI       | [`mask_contact_uri`]      |
//! | STATIC_BLOB_REPLACEMENT| [`static_blob_replacement`] |
//! | MASK_NETWORK_ID        | [`mask_network_id`]       |
//! | GENERALIZE_LOCATION    | [`generalize_location`]   |
//! | NOISY_DATE             | [`noisy_date`]            |
//! | STATIC_MAPPING         | [`static_mapping`]        |
//! | NOISY_POSITION         | [`noisy_position`]        |

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use chrono::{Duration, NaiveDate, NaiveDateTime};

// ─────────────────────────────────────────────────────────────────────────────
// Public dispatch
// ─────────────────────────────────────────────────────────────────────────────

/// Apply the masking strategy named `method` to a string `input`.
///
/// This is the single entry-point used by the anonymisation pipeline.
pub fn mask_value(method: &str, input: &str) -> String {
    match method {
        "PRESERVE_TOKEN" => preserve_token(input),
        "REDACT_ALPHANUMERIC" => redact_alphanumeric(input),
        "MASK_CONTACT_URI" => mask_contact_uri(input),
        "STATIC_BLOB_REPLACEMENT" => static_blob_replacement().to_string(),
        "MASK_NETWORK_ID" => mask_network_id(input),
        "GENERALIZE_LOCATION" => generalize_location(input),
        "NOISY_DATE" => noisy_date(input),
        "STATIC_MAPPING" => static_mapping(input),
        "NOISY_POSITION" => noisy_position(input),
        _ => input.to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PRESERVE_TOKEN
// ─────────────────────────────────────────────────────────────────────────────

/// Deterministic pseudonymisation: the same input always produces the same
/// output (referential integrity preserved across the dataset).
///
/// Character classes are preserved – uppercase stays uppercase, lowercase
/// stays lowercase, digits stay digits.  Separators and non-ASCII characters
/// are kept unchanged so structural patterns (e.g. `XXXX-YYYY`) remain.
pub fn preserve_token(input: &str) -> String {
    let h = stable_hash(input);
    input
        .chars()
        .enumerate()
        .map(|(i, c)| {
            // Per-position offset derived from the input hash.
            let offset = h.wrapping_add((i as u64).wrapping_mul(2_654_435_769)) as usize;
            if c.is_ascii_uppercase() {
                (b'A' + (offset % 26) as u8) as char
            } else if c.is_ascii_lowercase() {
                (b'a' + (offset % 26) as u8) as char
            } else if c.is_ascii_digit() {
                (b'0' + (offset % 10) as u8) as char
            } else {
                c
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// REDACT_ALPHANUMERIC
// ─────────────────────────────────────────────────────────────────────────────

/// Replace every ASCII letter with `X` and every ASCII digit with `9`;
/// punctuation, whitespace and Unicode characters are left unchanged.
///
/// Useful for IDs, serial numbers, licence plates, IBANs, etc.
pub fn redact_alphanumeric(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_digit() {
                '9'
            } else if c.is_alphabetic() {
                'X'
            } else {
                c
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// MASK_CONTACT_URI
// ─────────────────────────────────────────────────────────────────────────────

/// Mask an e-mail address, URL, or phone / fax number.
///
/// * **Email**  – keep the domain, anonymise the local part → `xxxx@domain.com`
/// * **URL**    – keep scheme + host, redact path/query    → `https://example.com/XXXXX`
/// * **Other**  – apply [`redact_alphanumeric`] (phones, fax numbers, …)
pub fn mask_contact_uri(input: &str) -> String {
    let t = input.trim();
    if t.contains('@') {
        mask_email(t)
    } else if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("ftp://") {
        mask_url(t)
    } else {
        redact_alphanumeric(t)
    }
}

fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first = local.chars().next().unwrap_or('x');
            format!("{first}xxx@{domain}")
        }
        None => redact_alphanumeric(email),
    }
}

fn mask_url(url: &str) -> String {
    // Keep "scheme://authority", replace everything after with a fixed token.
    let scheme_end = url.find("://").map(|p| p + 3).unwrap_or(0);
    let after_scheme = &url[scheme_end..];
    let authority_len = after_scheme.find('/').unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_len];
    format!("{}{}/XXXXX", &url[..scheme_end], authority)
}

// ─────────────────────────────────────────────────────────────────────────────
// STATIC_BLOB_REPLACEMENT
// ─────────────────────────────────────────────────────────────────────────────

/// Fixed placeholder for binary / large-object fields (photos, signatures,
/// fingerprints, voice prints, logos, …).
pub fn static_blob_replacement() -> &'static str {
    "[REDACTED]"
}

// ─────────────────────────────────────────────────────────────────────────────
// MASK_NETWORK_ID
// ─────────────────────────────────────────────────────────────────────────────

/// Anonymise an IPv4 address, IPv6 address, or MAC address.
///
/// * **IPv4** – zero the last octet            → `192.168.1.0`
/// * **MAC**  – zero the last three bytes (OUI preserved) → `aa:bb:cc:00:00:00`
/// * **IPv6** – zero the last four groups (full form only) → `2001:db8:0:1:0:0:0:0`
/// * **Other** – fall back to [`redact_alphanumeric`]
pub fn mask_network_id(input: &str) -> String {
    let t = input.trim();
    try_mask_ipv4(t)
        .or_else(|| try_mask_mac(t))
        .or_else(|| try_mask_ipv6(t))
        .unwrap_or_else(|| redact_alphanumeric(t))
}

fn try_mask_ipv4(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() == 4 && parts.iter().all(|p| p.parse::<u8>().is_ok()) {
        Some(format!("{}.{}.{}.0", parts[0], parts[1], parts[2]))
    } else {
        None
    }
}

fn try_mask_mac(input: &str) -> Option<String> {
    let sep = if input.contains(':') {
        ':'
    } else if input.contains('-') {
        '-'
    } else {
        return None;
    };
    let parts: Vec<&str> = input.split(sep).collect();
    if parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && u8::from_str_radix(p, 16).is_ok())
    {
        let sep_s = sep.to_string();
        let oui = [parts[0], parts[1], parts[2]].join(&sep_s);
        let suffix = ["00", "00", "00"].join(&sep_s);
        Some(format!("{oui}{sep}{suffix}"))
    } else {
        None
    }
}

fn try_mask_ipv6(s: &str) -> Option<String> {
    let groups: Vec<&str> = s.split(':').collect();
    if groups.len() == 8 {
        Some(format!(
            "{}:{}:{}:{}:0:0:0:0",
            groups[0], groups[1], groups[2], groups[3]
        ))
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GENERALIZE_LOCATION
// ─────────────────────────────────────────────────────────────────────────────

/// Generalise a postal / zip code to its regional prefix.
///
/// Keeps the first half of the code (rounded up) and replaces the rest with
/// `0` (digits) or `X` (letters), preserving separators.
///
/// Examples: `"10001"` → `"10000"`, `"SW1A 2AA"` → `"SW1X XXX"`
pub fn generalize_location(input: &str) -> String {
    let t = input.trim();
    if t.is_empty() {
        return t.to_string();
    }
    let keep = (t.len() + 1) / 2;
    t.chars()
        .enumerate()
        .map(|(i, c)| {
            if i < keep {
                c
            } else if c.is_ascii_digit() {
                '0'
            } else if c.is_alphabetic() {
                'X'
            } else {
                c
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// NOISY_dATE
// ─────────────────────────────────────────────────────────────────────────────

/// Add a deterministic ±30-day noise to a date string.
///
/// The noise is derived from the input via a stable hash, so the same input
/// always yields the same output (reproducible anonymisation).
///
/// Recognised formats:
/// * `YYYY-MM-DD`
/// * `YYYY-MM-DD HH:MM:SS`
/// * MongoDB extended: `YYYY-MM-DD H:MM:SS.f +HH:MM:SS`
///
/// Unrecognised inputs are returned unchanged.
pub fn noisy_date(input: &str) -> String {
    let t = input.trim();
    let noise = ((stable_hash(t) as i64) % 61) - 30; // deterministic [-30, +30] days
    let delta = Duration::days(noise);

    // 1. Bare date: YYYY-MM-DD
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return (d + delta).format("%Y-%m-%d").to_string();
    }

    // 2. Datetime with seconds: YYYY-MM-DD HH:MM:SS
    if let Ok(dt) = NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S") {
        return (dt + delta).format("%Y-%m-%d %H:%M:%S").to_string();
    }

    // 3. MongoDB extended: "2019-02-11 5:00:00.0 +00:00:00"
    //    Trim timezone offset, parse datetime, re-append offset.
    if let Some(tz_pos) = t.rfind(" +").or_else(|| t.rfind(" -")) {
        let dt_part = t[..tz_pos].trim();
        let tz_part = t[tz_pos..].trim();
        let parsed = NaiveDateTime::parse_from_str(dt_part, "%Y-%m-%d %H:%M:%S%.f")
            .or_else(|_| NaiveDateTime::parse_from_str(dt_part, "%Y-%m-%d %H:%M:%S"));
        if let Ok(dt) = parsed {
            return format!("{} {}", (dt + delta).format("%Y-%m-%d %H:%M:%S"), tz_part);
        }
    }

    input.to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// STATIC_MAPPING
// ─────────────────────────────────────────────────────────────────────────────

/// Deterministically map a categorical value (civility, gender, age class, …)
/// to an anonymous token from a fixed alphabet.
///
/// The same input always maps to the same token so joins across fields remain
/// consistent.
pub fn static_mapping(input: &str) -> String {
    const TOKENS: &[&str] = &["A", "B", "C", "D", "E"];
    let idx = (stable_hash(input) as usize) % TOKENS.len();
    TOKENS[idx].to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// NOISY_POSITION
// ─────────────────────────────────────────────────────────────────────────────

/// Add a small, deterministic noise (~±0.009°, roughly ±1 km) to a geographic
/// coordinate string.
///
/// Accepted formats: `"lat,lon"`, `"lat lon"`, or a single floating-point
/// value.  The output uses six decimal places.
pub fn noisy_position(input: &str) -> String {
    let t = input.trim();

    let sep = if t.contains(',') {
        Some(',')
    } else if t.contains(' ') {
        Some(' ')
    } else {
        None
    };

    if let Some(s) = sep {
        let mut parts = t.splitn(2, s);
        if let (Some(lat_s), Some(lon_s)) = (parts.next(), parts.next()) {
            if let (Ok(lat), Ok(lon)) = (lat_s.trim().parse::<f64>(), lon_s.trim().parse::<f64>()) {
                let lat = lat + coord_noise(&format!("lat:{t}"));
                let lon = lon + coord_noise(&format!("lon:{t}"));
                return format!("{lat:.6},{lon:.6}");
            }
        }
    }

    if let Ok(v) = t.parse::<f64>() {
        return format!("{:.6}", v + coord_noise(t));
    }

    input.to_string()
}

/// Deterministic noise bounded to ±0.009° (~1 km).
fn coord_noise(seed: &str) -> f64 {
    let raw = (stable_hash(seed) as i64).wrapping_rem(1_000);
    raw as f64 * 0.009 / 1_000.0
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Stable, deterministic hash of a string (uses `DefaultHasher`).
fn stable_hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}
