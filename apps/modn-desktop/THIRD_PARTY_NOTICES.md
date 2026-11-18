# Third-party notices

Modn Desktop uses Tauri 2 and its official crates, including `tauri`,
`tauri-build` and `tauri-plugin-dialog`, plus `clipboard-win` for the
Rust-side clipboard write, and their transitive dependencies. Tauri is
licensed under Apache-2.0 OR MIT. `clipboard-win` is licensed under BSL-1.0.
The root workspace `Cargo.lock` records the full dependency graph and versions.
A distributor must audit the locked dependency graph and retain the notices
that the included licenses require.
This source notices file does not certify a complete binary distribution
license audit.
