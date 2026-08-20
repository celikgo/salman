// SPDX-License-Identifier: Apache-2.0
//! The single source of version truth.
//!
//! The `VERSION` file beside this crate's manifest is authoritative. This
//! module embeds it at compile time and asserts — also at compile time — that
//! it agrees with the version Cargo was given. The two therefore cannot drift:
//! a mismatch is a build failure on every machine, not a CI job that someone
//! might skip.
//!
//! The file lives inside this crate rather than at the repository root so that
//! it travels in the published `.crate` tarball. An `include_str!` reaching
//! above the package directory compiles in a git checkout and fails for every
//! person who installs from crates.io, which would make the guarantee hold
//! only for the people who least need it. See
//! `docs/adr/ADR-0008-one-version-truth.md`.

/// Raw contents of the `VERSION` file, including its trailing newline.
const VERSION_FILE: &str = include_str!("../VERSION");

/// The salman version string, e.g. `"0.1.0"`.
///
/// Read from the `VERSION` file, not from Cargo metadata.
pub const VERSION: &str = trim_ascii_end(VERSION_FILE);

/// Compile-time proof that `VERSION` and Cargo's package version agree.
const _: () = assert!(
    str_eq(VERSION, env!("CARGO_PKG_VERSION")),
    "the VERSION file disagrees with the version in Cargo.toml"
);

/// Trims trailing ASCII whitespace in a `const` context.
///
/// `str::trim_end` is not `const`, and this runs at compile time.
const fn trim_ascii_end(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    while end > 0 {
        match bytes[end - 1] {
            b'\n' | b'\r' | b' ' | b'\t' => end -= 1,
            _ => break,
        }
    }
    let (head, _) = bytes.split_at(end);
    match core::str::from_utf8(head) {
        Ok(s) => s,
        // Only ASCII whitespace was removed from the end, so a UTF-8 error
        // here means the VERSION file itself is not valid UTF-8.
        #[allow(clippy::panic)]
        Err(_) => panic!("VERSION file is not valid UTF-8"),
    }
}

/// String equality in a `const` context.
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_is_read_from_the_version_file_and_matches_cargo() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn version_has_no_surrounding_whitespace() {
        assert_eq!(VERSION.trim(), VERSION);
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn version_is_three_dotted_numeric_components() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "VERSION must be MAJOR.MINOR.PATCH, got {VERSION:?}"
        );
        for p in parts {
            assert!(
                !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()),
                "VERSION component {p:?} is not numeric"
            );
        }
    }
}
