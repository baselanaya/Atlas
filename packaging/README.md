# Publishing Atlas

## AUR (Arch User Repository)

The PKGBUILD here builds Atlas from a released tag. To publish it as
`atlas-companion`:

1. Create an AUR account, and register an SSH key with it
   (https://aur.archlinux.org/ → My Account).
2. Clone the empty package name once:
   ```bash
   git clone ssh://aur@aur.archlinux.org/atlas-companion.git
   ```
3. Copy `PKGBUILD` in, fill the real `sha256sums`
   (`curl -sL <tarball-url> | sha256sum`), generate and commit the metadata:
   ```bash
   makepkg --printsrcinfo > .SRCINFO
   git add PKGBUILD .SRCINFO
   git commit -m "atlas-companion 0.3.0"
   git push
   ```
4. Test the build in a clean chroot before pushing:
   ```bash
   paru -G atlas-companion 2>/dev/null || true
   makechrootpkg -c -r $HOME/chroot
   ```

Notes for future versions: bump `pkgver`/`pkgrel`, keep `source` pointing at
the tag's tarball, and never ship the AppImage inside the package — the whole
point of the AUR build is compiling from source against the user's own
webkit2gtk.

## Release checklist (maintainer)

1. `CHANGELOG.md` — add the version's section.
2. Bump `version` in `Cargo.toml`, `src-tauri/tauri.conf.json`, `package.json`.
3. `npm run pack` locally to prove the build, refresh `~/Applications` if desired.
4. Commit, push, wait for CI green.
5. `git tag vX.Y.Z && git push origin vX.Y.Z` — the Release workflow builds
   and attaches deb + AppImage + Windows setup.
6. Update the AUR package (above) to the new tag.
