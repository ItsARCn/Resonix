# Resonix Release Guide

This guide walks through cutting and publishing a new Resonix release on GitHub (`ItsARCn/Resonix`) from start to finish.

Everything runs automatically through GitHub Actions once you push a `vX.Y.Z` tag, **as long as the 4 preparation steps below are committed to `main` first**.

---

## Before Your First Release (One-Time Setup)

Make sure these two things exist (already done if you set up your signing keys):

1. [`assets/update-public-key.hex`](assets/update-public-key.hex) contains your 64-character Ed25519 **public** key.
2. **GitHub → Settings → Secrets and variables → Actions** has `RESONIX_UPDATE_SIGNING_KEY` set to the matching 64-character **private** key.

> You do **not** need Azure or Apple paid signing accounts. GitHub Actions builds and ships unsigned Windows (`.msi`/`.zip`) and ad-hoc signed macOS (`.dmg`) packages automatically.

---

## Step-by-Step: Publishing a New Version (e.g. `0.18.0`)

Pick your new version number (for example `0.18.0`). For pre-releases/test builds, add a suffix like `0.18.0-rc.1` (pre-releases stay off the `latest` release so auto-updater users only get stable updates).

### Step 1: Update the version in `Cargo.toml` and `Cargo.lock`

1. Open [`Cargo.toml`](Cargo.toml) and change line 4:
   ```toml
   version = "0.18.0"
   ```
2. Update [`Cargo.lock`](Cargo.lock) to match by running:
   ```sh
   cargo check
   ```

> **Important:** The release workflow refuses to build if the `v0.18.0` Git tag and `Cargo.toml`'s `version` do not match.

---

### Step 2: Add the release to Flatpak metainfo

Open [`packaging/flatpak/io.github.itsarcn.Resonix.metainfo.xml`](packaging/flatpak/io.github.itsarcn.Resonix.metainfo.xml) and add the new `<release>` entry **at the top** of `<releases>` (matching your version and today's `YYYY-MM-DD` date):

```xml
  <releases>
    <release version="0.18.0" date="2026-12-01">
      <url>https://github.com/ItsARCn/Resonix/releases/tag/v0.18.0</url>
    </release>
    <!-- keep older entries below -->
  </releases>
```

---

### Step 3: Write the release notes file

Create `packaging/release-notes/v0.18.0.md` (matching the exact `v` + version filename!).  
GitHub Actions uses this file directly as the release description instead of auto-generating notes.

Starter template (replace `0.18.0` and the bullet points):

```markdown
Resonix 0.18.0 brings ...

**Download Resonix:** [Mac](https://github.com/ItsARCn/Resonix/releases/download/v0.18.0/resonix-v0.18.0-macos-universal.dmg) · [Windows](https://github.com/ItsARCn/Resonix/releases/download/v0.18.0/resonix-v0.18.0-x86_64-pc-windows-msvc.msi) · [Windows ARM](https://github.com/ItsARCn/Resonix/releases/download/v0.18.0/resonix-v0.18.0-aarch64-pc-windows-msvc.msi) · [Linux](https://github.com/ItsARCn/Resonix/releases/download/v0.18.0/resonix-v0.18.0-x86_64-unknown-linux-gnu.tar.gz) · [Linux ARM](https://github.com/ItsARCn/Resonix/releases/download/v0.18.0/resonix-v0.18.0-aarch64-unknown-linux-gnu.tar.gz) · [Flatpak](https://github.com/ItsARCn/Resonix/releases/download/v0.18.0/resonix-v0.18.0-x86_64.flatpak) · [AppImage](https://github.com/ItsARCn/Resonix/releases/download/v0.18.0/resonix-0.18.0-x86_64.AppImage) · [AppImage ARM](https://github.com/ItsARCn/Resonix/releases/download/v0.18.0/resonix-0.18.0-aarch64.AppImage)

## New

- **Feature title.** Short description of what changed for the user.

## Fixed

- **Bug fix title.** Short description of what was fixed.

## Thanks

Thanks to everyone testing and reporting issues!

**Full changelog**: https://github.com/ItsARCn/Resonix/commits/v0.18.0
```

> Notice AppImage filenames omit the `v` (`resonix-0.18.0-x86_64.AppImage`), while `.dmg`, `.msi`, `.zip`, `.flatpak`, and `.tar.gz` include `v` (`resonix-v0.18.0-...`).  
> Linux `.deb` and `.rpm` packages are attached automatically right after the release publishes (`resonix_0.18.0_amd64.deb`, `resonix-0.18.0-1.x86_64.rpm`, etc.).

---

### Step 4: Run local checks before committing

Run these commands to catch any typo, missing file, or formatting error before GitHub Actions spends build time:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
python3 packaging/test-release-names.py
python3 packaging/flatpak/test-metainfo.py
appstreamcli validate --no-net packaging/flatpak/io.github.itsarcn.Resonix.metainfo.xml
```

> `test-release-names.py` checks that `packaging/release-notes/vVERSION.md` exists and is non-empty for the version in `Cargo.toml`.

---

### Step 5: Commit, push `main`, then create & push the tag

Always commit and push `main` **before** creating the tag so the release build has your bumped version and release notes:

```sh
git add Cargo.toml Cargo.lock packaging/flatpak/io.github.itsarcn.Resonix.metainfo.xml packaging/release-notes/v0.18.0.md
git commit -m "Release v0.18.0"
git push origin main
```

Now create the tag and push it to start the release build:

```sh
git tag v0.18.0
git push origin v0.18.0
```

---

## What Happens Automatically on GitHub Actions

Once `v0.18.0` is pushed, watch **GitHub → Actions**:

1. **Release workflow ([`.github/workflows/release.yml`](.github/workflows/release.yml))** builds:
   - Linux `x86_64` & `aarch64` `.tar.gz` + `.AppImage` + `.AppImage.zsync`
   - Windows `x64` & `arm64` `.zip` (portable) + `.msi` (installer)
   - macOS universal `.dmg`
   - Linux `x86_64` `.flatpak` bundle
   - Source tarball (`resonix-v0.18.0-source.tar.gz`)
   - Generates `checksums.txt`, signs it into `checksums.txt.sig` using `RESONIX_UPDATE_SIGNING_KEY`, verifies the signature against [`assets/update-public-key.hex`](assets/update-public-key.hex), and publishes the GitHub Release with your release notes.
2. **Packaging workflow ([`.github/workflows/packaging.yml`](.github/workflows/packaging.yml))** runs automatically after the release publishes (for stable tags without `-`) to build and attach:
   - `resonix_0.18.0_amd64.deb` & `resonix_0.18.0_arm64.deb`
   - `resonix-0.18.0-1.x86_64.rpm` & `resonix-0.18.0-1.aarch64.rpm`
   - `packaging-checksums.txt`
   - Installs and smoke-tests the `.deb`/`.rpm` inside Ubuntu, Debian, and Fedora containers.

---

## Step 6: After the Release Publishes (Optional)

- Check your release page on `https://github.com/ItsARCn/Resonix/releases` and confirm all files, `checksums.txt`, `checksums.txt.sig`, and `packaging-checksums.txt` are present.
- Existing Resonix installs will detect the new version automatically via `checksums.txt` + `checksums.txt.sig`.
- If you also update docs version metadata for a GitHub Pages site, update [`docs/_config.yml`](docs/_config.yml) and [`docs/_data/versions.yml`](docs/_data/versions.yml) **after** the GitHub Release is live.

---

## Quick Troubleshooting

- **Tag failed because of a typo or build issue before publishing?**  
  Delete the bad tag locally and on GitHub, fix the issue on `main`, and re-tag:
  ```sh
  git tag -d v0.18.0
  git push origin :refs/tags/v0.18.0
  ```
- **Did `Sign checksums` fail?**  
  Verify `RESONIX_UPDATE_SIGNING_KEY` is set in **Repo → Settings → Secrets and variables → Actions** and matches [`assets/update-public-key.hex`](assets/update-public-key.hex).
- **Never rotate/delete `RESONIX_UPDATE_SIGNING_KEY` unnecessarily:** existing installed copies check updates against the public key compiled into them (`assets/update-public-key.hex`).
