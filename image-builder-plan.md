Goal: a Rust fluent-API image builder, e.g.

```rust
Image::base("alpine")
    .run("apk add python3")
    .copy("./app", "/app")
    .build("myimage");
```

then `rcr run --image myimage python3 /app/script.py` to execute real code.

---

## Phase 1 — OverlayFS (foundation)

1. Replace the direct `pivot_root` into shared `ROOTFS` with an overlay mount: read-only `lowerdir` (base image) + per-container `upperdir` + `workdir`, then pivot into the merged view.
2. **Verify:** changes inside a container land in its upper layer, the base stays clean, and two containers don't see each other's writes.

## Phase 2 — Commit (snapshot a layer)

3. Add the ability to run a command in a container, then capture its `upperdir` as a new read-only layer (a directory holding just the diff).
4. **Verify:** run `apk add python3`, snapshot, confirm the new layer contains only the changed files.

## Phase 3 — Layer stacking

5. Support an overlay with _multiple_ lowerdirs (base + each committed layer stacked), so a container can run on top of N layers.
6. **Verify:** stack two layers, run a container, see both layers' changes present.

## Phase 4 — The fluent builder

7. Define `Image` with chained methods (`base`, `run`, `copy`, `env`) that each push a `BuildStep` onto a `Vec`.
8. `build(name)` executes steps in order: for each, run a container on the current layers, apply the step, commit the diff as a new layer.
9. Store the finished image as an ordered list of layer paths under an image name.

## Phase 5 — Run an image

10. `rcr run --image myimage ...` mounts the image's stacked layers as the container root (replacing the hardcoded `ROOTFS`).
11. **Verify the end goal:** build an image with Python + your code via the fluent API, then run it and execute real code.

## Phase 6 — Caching (optional)

12. Hash each step; skip rebuilding layers whose step + inputs are unchanged.

---

**Critical path:** Phases 1–5. Caching is polish.
Each phase is independently testable and builds on the last.

**Next step:** Phase 1 — OverlayFS.
