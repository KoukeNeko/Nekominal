Release notes, read by the release workflow.

- `release-notes/vX.Y.Z.md`: the GitHub release's notes for tag `vX.Y.Z`, in English and then in Traditional Chinese
  (a release has one body, so both are in the one file, the same sections in the same order).
  Without one, the release falls back to notes generated from the commits.
  Written before the tag is pushed; a run again on a release that exists puts the file's notes on it as well.

Nekominal is published only on GitHub and in the Scoop bucket (`KoukeNeko/scoop-bucket`), so there is no store
"What's New" to keep; the Scoop manifest is written by `scripts/render-scoop-manifest.ps1`.
