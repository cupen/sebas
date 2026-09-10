## 1. Spec repair (applied directly to openspec/specs/core-session-channel/spec.md)

- [ ] 1.1 Remove the malformed duplicate「Session drive methods」block + orphan scenario lines (done).
- [ ] 1.2 Fix default socket path `~/.sebas/core.sock` → `$XDG_RUNTIME_DIR/sebas/core.sock` + per-uid temp fallback (done).
- [ ] 1.3 Fix ensure-resume scenario: `Revived` → `Updated` (done).

## 2. Validation

- [ ] 2.1 `openspec validate --specs` passes for core-session-channel (done — 0 invalid).
