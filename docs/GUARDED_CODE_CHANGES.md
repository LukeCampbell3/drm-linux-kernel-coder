# Guarded code changes

`code.patch` lets an explicitly authorized DRM instance apply a unified Git patch to its codebase, run a fixed verification command, and retain the change only when verification succeeds. It is disabled by default, including in the desktop image.

## Enable it for one repository

Add a user-service override with an exact repository root and the narrowest useful path list:

```ini
[Service]
Environment=DRMD_CODE_ROOT=/home/drm/projects/my-app
Environment=DRMD_CODE_ALLOWED_PATHS=src,tests,docs
Environment=DRMD_CODE_MAX_PATCH_BYTES=262144
Environment=DRMD_CODE_VERIFY_PROGRAM=cargo
Environment="DRMD_CODE_VERIFY_ARGS=test --workspace"
```

Then place a unified patch under the instance work directory and submit it:

```bash
drmd submit --task repair_parser \
  --ops code.patch,notify.send \
  --source proposals/repair-parser.patch
```

The daemon never accepts an arbitrary shell command from an episode. The operator fixes the verifier executable and arguments when configuring the service.

## Guardrails

- The configured root must resolve to the exact root of a Git worktree.
- Every patch target must be relative and inside `DRMD_CODE_ALLOWED_PATHS`.
- `.git`, parent traversal, absolute paths, symlink components, binary patches, and dirty target files are rejected.
- Deletion is disabled unless `DRMD_CODE_ALLOW_DELETE=1` is explicitly configured.
- Patch size is bounded and `git apply --check --whitespace=error-all` must pass before mutation.
- The fixed verifier runs after application. A failed or unavailable verifier reverses the patch automatically.
- A rollback failure is reported prominently and never counted as a successful code change.

These controls bound where and how an instance can edit. Verification quality still depends on the configured test command; use the strongest deterministic suite practical for the repository.
