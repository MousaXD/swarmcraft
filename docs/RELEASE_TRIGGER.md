# v0.1.0-preview release trigger

The preview release is cut from merged `main` only after the permanent CI matrix and release gates are green. Release artifacts are rebuilt from the tag by `.github/workflows/release.yml`.
