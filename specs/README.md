# Specs

Behaviour specifications written by the **spec agent** from observing the
real OS, one file per entry in the coverage map (`coverage/`):

```
specs/<os>/<component>/<symbol>.yaml      spec sheet (see TEMPLATE.yaml)
specs/<os>/<component>/<symbol>.tests/    visible tests (~70%)
```

Held-out tests (~30%) are **not** stored in this repository. They live in
the held-out store (`$POLY_HELDOUT_DIR`, default `~/.poly-heldout`) with the
diff runner, so implementer agents can never see them. See `../CLEAN_ROOM.md`.

Every hand-off from spec agent to implementer is appended to `HANDOFF_LOG.md`.

The tools for all of this are `cargo xtask pipeline spec new`, `tests split`
and `handoff record|bundle|verify`; see `../docs/AGENT_PIPELINE.md`.
