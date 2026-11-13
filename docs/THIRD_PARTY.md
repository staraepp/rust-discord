# Implementation references and notices

The native text input in `dc-ui/src/input.rs` is adapted from GPUI 0.2.2’s
`examples/input.rs` (Zed Industries and contributors), licensed Apache-2.0.
The full license is in `licenses/GPUI-APACHE-2.0.txt`. Our changes add submit,
undo/redo, dark styling, public editor integration, and IME range fixes.

Serein (MIT OR Apache-2.0) was reviewed as an architecture/dependency reference.
Its emulation/fingerprinting transport is not used. Discord’s libdave is
MIT-licensed; its integration remains subject to the project’s dependency
and cryptography rules. No Vencord or proprietary Discord client code is copied.

Development packages include the project license, the retained GPUI input
license, and a generated `THIRD_PARTY_LICENSES.txt`. Run
`cargo xtask licenses <output-path>` to collect notices included with the
resolved crates for the host platform, including bundled native sources.
The collector uses offline Cargo metadata; dependency versions remain in
Cargo.lock. cargo-deny remains the dependency policy check.
