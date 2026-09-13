# Supplemental license notices

These files supply notices omitted from published Cargo packages. Each directory
matches a package name and version in `Cargo.lock`; `SOURCE.txt` records the upstream
source. Most sources use the commit recorded in the crate's `.cargo_vcs_info.json`.
The simd_helpers 0.1.0 source commit predates its LICENSE file, so that directory
includes the license subsequently published by its upstream project.

`scripts/package-desktop.py` uses the license files bundled with each Cargo crate
first, and falls back to these copies only when the crate has none. They are
third-party notices, not a license for LocalVoice's own code.
