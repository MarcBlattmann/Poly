# 0163: The macOS framework map credits what a library re-exports

**Status:** Accepted. Refines how `coverage import macos-sdk` reads the SDK's
`.tbd` stubs (ADR 0004, 0006). Found by the Chrome import map (ADR 0157).

## Context

The macOS part of the coverage map is seeded from the SDK's text stubs
(`.tbd`). Chrome for macOS imports about 120 symbols that the map did not
have for the library they come from, among them `NSFontAttributeName`
(AppKit), `AudioConverterNew` (AudioToolbox), every AVFoundation symbol,
and `memcpy` and `strlen` (libSystem). The SDK has all of them. The seed
missed them in two places:

- A stub of a public library also holds, as further documents, the private
  libraries it re-exports (AppKit's file holds UIFoundation and
  CollectionViewCore, AudioToolbox's AudioToolboxCore, AVFoundation's
  AVFCore and AVFCapture). The seed drops private libraries, so their
  symbols were lost, although apps link them through the public library.
  AVFoundation had no component at all.
- A stub lists symbols a library makes available from libraries it
  re-exports under `reexports:` (libsystem_c's `memcpy`, `strlen`, ...).
  The seed read `exports:` only.

## Decision

- The documents after the first one in a `.tbd` file are credited to the
  file's first library when they are private: an app sees them as that
  public library's API. Public libraries inlined in a file keep their own
  component, as before.
- `reexports:` lists (YAML, version 4) and `reexported_symbols` (JSON,
  version 5) count as the library's API, like `exports:`.
- The map was re-seeded from the same SDK (macOS SDK 27.0, 26A425): 28,652
  entries added (741,217 macOS framework API entries in all), five new
  components (`AVFoundation.framework`, `AudioUnit.framework`,
  `PassKit.framework`, `libktrace.dylib`, `liblaunch.dylib`). No entry was
  removed and no status changed.

## Consequences

- A symbol may now be listed under two components (where it is defined and
  where it is re-exported). The map counts API an app can link against, so
  that is what it should count.
- The Chrome import map's "not in the map" column shrinks to what is really
  private (one weak import of SkyLight) once this is merged and the report
  regenerated.
