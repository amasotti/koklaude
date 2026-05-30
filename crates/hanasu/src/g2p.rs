//! Grapheme-to-phoneme via the espeak-ng CLI.

use std::process::Command;

use crate::{Error, Result};

/// Phonemize English text to IPA (en-us) via espeak-ng.
///
/// The caller passes a punctuation-free chunk; punctuation/pause handling lives
/// in the tokenizer. espeak drops punctuation and emits a newline on clause
/// breaks, so `normalize` flattens any whitespace back to single spaces.
///
/// The `--` is required: without it espeak parses text starting with `-` (e.g. a
/// list dash or a negative number) as an option and silently emits nothing.
pub(crate) fn phonemize(text: &str) -> Result<String> {
    let output = Command::new("espeak-ng")
        .args(["-q", "--ipa", "-v", "en-us", "--", text])
        .output()
        .map_err(|e| Error::Espeak(format!("could not run espeak-ng: {e}")))?;

    if !output.status.success() {
        return Err(Error::Espeak(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let raw = String::from_utf8(output.stdout).map_err(|e| Error::Espeak(e.to_string()))?;
    Ok(normalize(&raw))
}

fn normalize(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_whitespace_and_clause_newlines() {
        assert_eq!(normalize("həlˈoʊ wˈɜːld\n"), "həlˈoʊ wˈɜːld");
        assert_eq!(normalize("ɡˈʊd\nðə bˈɪld"), "ɡˈʊd ðə bˈɪld");
        assert_eq!(normalize("  spaced   out \n"), "spaced out");
    }

    #[test]
    fn normalize_handles_empty() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("  \n "), "");
    }

    fn espeak_available() -> bool {
        Command::new("espeak-ng").arg("--version").output().is_ok()
    }

    #[test]
    fn phonemize_matches_espeak_fixtures() {
        if !espeak_available() {
            eprintln!("skipping phonemize_matches_espeak_fixtures: espeak-ng not installed");
            return;
        }
        assert_eq!(phonemize("Hello world").unwrap(), "həlˈoʊ wˈɜːld");
        assert_eq!(phonemize("Kubernetes").unwrap(), "kˌuːbɚnˈɛɾiːz");
        assert_eq!(phonemize("").unwrap(), "");
    }

    #[test]
    fn phonemize_handles_leading_dash() {
        if !espeak_available() {
            eprintln!("skipping phonemize_handles_leading_dash: espeak-ng not installed");
            return;
        }
        // Without `--`, espeak treats these as options and returns nothing.
        assert!(!phonemize("-5 degrees").unwrap().is_empty());
        assert!(!phonemize("- a list item").unwrap().is_empty());
    }
}
