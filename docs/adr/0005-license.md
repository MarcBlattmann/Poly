# 0005: License

**Status:** Accepted (2026-10-20, by the project owner)

## Decision

Poly is licensed under the Apache License 2.0 only (see `LICENSE`).
Every contribution is accepted under the same license (section 5 of the license).

## Why

- Permissive: anyone can use, change and ship it, which suits an OS that wants adoption.
- Explicit patent grant from every contributor, with automatic termination
  for anyone who sues over patents in the code. This protects users and the
  project from contributors who later assert patents.
- It keeps the clean-room boundary simple: no GPL or LGPL code may enter, and
  there is no copyleft on our side that would raise questions about which
  combinations can be distributed.

## Alternatives considered

- **MIT OR Apache-2.0** (the Rust ecosystem default): rejected to keep a single
  license with the patent grant always in force.
- **GPL-2.0 or later**: forces derivatives to stay open, but discourages
  adoption and does not make the clean-room process any easier.
