//! Names of visuals the user creates, and of the files they're exported to.

/// Longest name kept, in characters.
const MAX_LENGTH: usize = 40;

/// A clean, unused visual name based on `wanted`: on one line, trimmed and
/// shortened, then numbered if taken ("glow", "glow 2", "glow 3"…). Names are
/// compared ignoring case. `None` if nothing is left once cleaned.
pub fn unique_name(wanted: &str, taken: &[String]) -> Option<String> {
    let base: String = wanted
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_LENGTH)
        .collect();
    let base = base.trim();
    if base.is_empty() {
        return None;
    }
    let is_taken = |name: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(name));
    if !is_taken(base) {
        return Some(base.to_owned());
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !is_taken(name))
}

/// The name of a shader file without its extension: `glow.wgsl` → `glow`.
pub fn name_from_file(file_name: &str) -> &str {
    file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
}

/// A file name for exporting a visual: `<name>.wgsl`, without characters
/// that file systems refuse.
pub fn file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if r#"\/:*?"<>|"#.contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!("{safe}.wgsl")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn numbers_taken_names() {
        let taken = names(&["radial", "bars", "Glow", "glow 2"]);
        assert_eq!(unique_name("tunnel", &taken).unwrap(), "tunnel");
        assert_eq!(unique_name("glow", &taken).unwrap(), "glow 3");
        assert_eq!(unique_name("Radial", &taken).unwrap(), "Radial 2");
    }

    #[test]
    fn cleans_names() {
        assert_eq!(unique_name("  my\n\tglow  ", &[]).unwrap(), "my glow");
        assert_eq!(
            unique_name(&"x".repeat(100), &[]).unwrap().len(),
            MAX_LENGTH
        );
        assert_eq!(unique_name(" \n ", &[]), None);
    }

    #[test]
    fn file_names() {
        assert_eq!(name_from_file("glow.wgsl"), "glow");
        assert_eq!(name_from_file("my.glow.wgsl"), "my.glow");
        assert_eq!(name_from_file("glow"), "glow");
        assert_eq!(file_name("a/b: c?"), "a_b_ c_.wgsl");
    }
}
