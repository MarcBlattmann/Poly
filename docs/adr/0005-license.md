# 0005: License

**Status:** Proposed (needs the project owner's confirmation)

## Decision

Dual-license under MIT OR Apache-2.0, the Rust ecosystem default.

## Why

- Permissive: anyone can embed or ship it, which suits an OS that wants adoption.
- Apache-2.0 adds an explicit patent grant.
- It keeps the clean-room boundary simple: no GPL or LGPL code may enter, and
  there is no copyleft on our side that would raise questions about which
  combinations can be distributed.

## Alternative

GPL-2.0 or later, to force derivatives to stay open. Reading GPL sources would
still be limited to behaviour (see CLEAN_ROOM.md), so it would not make the
clean-room process any easier.
