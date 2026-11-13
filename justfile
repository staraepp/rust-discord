# Thin wrapper; the source of truth is `cargo xtask` (works without `just`).
ci:
    cargo xtask ci
fmt:
    cargo xtask fmt
clippy:
    cargo xtask clippy
test:
    cargo xtask test
deny:
    cargo xtask deny
