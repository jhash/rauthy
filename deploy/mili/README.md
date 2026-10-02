# Rauthy for mili

The `mili` branch of [jhash/rauthy](https://github.com/jhash/rauthy) is upstream
[Rauthy](https://github.com/sebadob/rauthy) `v0.36.2` plus patches mili needs. Each patch is
its own topic branch off the tag, merged into `mili`, so it can become an upstream pull request
on its own:

| Branch | Change |
|---|---|
| `fix/session-user-switch` | a login never reuses or overwrites another user's session |
| `feat/theme-custom-css` | `THEME_CUSTOM_DIR`: custom CSS appended to every theme, fonts served from it |
| `feat/client-theme-register-reset` | registration and password pages use the client's theme |

Only this directory is mili-specific:

- `Dockerfile` builds the WASM modules, the UI and an x86_64 release binary in one image,
  on the upstream builder image. `platforms` limits the proxbox CI build to `linux/amd64`.
- `theme/` is copied to `/app/theme` and `THEME_CUSTOM_DIR` points at it. `custom.css` gives
  Rauthy's pages the mili font stack, card and button look; its values follow mili's
  `design/tokens.json` (families, `radius`, `elevation`). The colours come from the themes that
  proxbox `scripts/identity-theme.sh` provisions, not from this file. Lora is under the SIL Open
  Font License in `theme/OFL.txt`.

Rebasing onto a new upstream release: rebase every topic branch onto the new tag, recreate
`mili` from the tag, merge the topic branches, then cherry-pick the `deploy/mili` commit.
