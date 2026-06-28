//! Live macOS system accent color.
//!
//! Read from the `AppleAccentColor` global default — the swatch the user picks
//! in System Settings ▸ Appearance. When the key is absent the user is on the
//! "multicolor" default; we return `None` so the app keeps its own brand blue
//! rather than forcing a specific shade.

/// Map an `AppleAccentColor` index to its control-accent hex. Returns `None`
/// for indices outside Apple's known swatch set. Values are Apple's standard
/// accent colors.
pub fn accent_hex_for_index(idx: i64) -> Option<&'static str> {
    Some(match idx {
        -1 => "#98989d", // Graphite
        0 => "#ff5257",  // Red
        1 => "#f7821b",  // Orange
        2 => "#febc2e",  // Yellow
        3 => "#62ba46",  // Green
        4 => "#007aff",  // Blue
        5 => "#a550a7",  // Purple
        6 => "#f74f9e",  // Pink
        _ => return None,
    })
}

/// The current system accent as a `#rrggbb` string, or `None` when unset
/// (multicolor default) or unreadable.
#[cfg(target_os = "macos")]
pub fn system_accent() -> Option<String> {
    // Absolute path documents the dependency and avoids any PATH edge case.
    // Under App Sandbox `spawn` is denied and `.ok()?` collapses to None — which
    // is the same brand-blue fallback as "multicolor", so the app degrades
    // gracefully (accent just won't track the system there).
    let out = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleAccentColor"])
        .output()
        .ok()?;
    // A non-zero exit means the key doesn't exist → user is on multicolor.
    if !out.status.success() {
        return None;
    }
    let idx = String::from_utf8(out.stdout)
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()?;
    accent_hex_for_index(idx).map(str::to_string)
}

#[cfg(not(target_os = "macos"))]
pub fn system_accent() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::accent_hex_for_index;

    #[test]
    fn known_indices_map_to_hex() {
        assert_eq!(accent_hex_for_index(4), Some("#007aff")); // Blue
        assert_eq!(accent_hex_for_index(-1), Some("#98989d")); // Graphite
        assert_eq!(accent_hex_for_index(6), Some("#f74f9e")); // Pink
    }

    #[test]
    fn unknown_index_is_none() {
        assert_eq!(accent_hex_for_index(99), None);
        assert_eq!(accent_hex_for_index(-5), None);
    }

    #[test]
    fn every_known_hex_is_well_formed() {
        for i in -1..=6 {
            let hex = accent_hex_for_index(i).expect("known index");
            assert_eq!(hex.len(), 7, "{i} → {hex}");
            assert!(hex.starts_with('#'));
            assert!(hex[1..].chars().all(|c| c.is_ascii_hexdigit()));
        }
    }
}
