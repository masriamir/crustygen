# Security Policy

## Supported versions

This project is pre-1.0.0 and `publish = false`; only `main` is supported — security fixes land there directly.

## Reporting a vulnerability

Please use [private vulnerability reporting](https://github.com/masriamir/crustygen/security/advisories/new) instead of filing a public issue for a suspected vulnerability.

## Security posture

crustygen is an offline command-line tool: it reads a hand-authored IR / map-spec and WAD files from the local file system and writes a PWAD; it makes no network requests and has no server component. Parsing of untrusted WAD input is delegated to [crustywad](https://github.com/masriamir/crustywad), consumed as a pinned crates.io release, whose hardening against malformed input is documented there in [ADR-0016, *Parser hardening policy*](https://github.com/masriamir/crustywad/blob/main/docs/adr/0016-parser-hardening-policy.md). The crustygen crate contains no `unsafe` code.

Supply chain: every third-party GitHub Action is pinned to a full commit SHA, and `cargo deny check` runs in CI as the required `security-deny` check.
