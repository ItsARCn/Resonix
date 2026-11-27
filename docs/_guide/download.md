---
title: Download
description: Download the app for macOS, Windows, or Linux, with install instructions for each.
nav_order: 1
seo:
  title: Download for Linux, macOS, and Windows
---

{% assign v = site.resonix_version %}
{% assign base = "https://github.com/ItsARCn/Resonix/releases/download/v" | append: v %}
{% comment %} Releases after 0.12.0 ship a Windows Installer package (.msi). {% endcomment %}
{% assign version = v | split: "." %}
{% assign major = version[0] | plus: 0 %}
{% assign minor = version[1] | plus: 0 %}
{% assign patch = version[2] | plus: 0 %}
{% assign windows_installer = "-setup.exe" %}
{% if major > 0 or minor > 12 or minor == 12 and patch > 0 %}{% assign windows_installer = ".msi" %}{% endif %}

The current version is **v{{ v }}**. You can use the
[download checksums]({{ base }}/checksums.txt) to check that a file has not
been damaged or changed; the DEB and RPM packages are listed in the
[package checksums]({{ base }}/packaging-checksums.txt). Older versions are on
the [releases page](https://github.com/ItsARCn/Resonix/releases).

| System | Ways to install |
| --- | --- |
| **macOS** (Apple Silicon and Intel) | [Disk image](#macos), [Homebrew](#macos), [Nix](#nix-darwin) |
| **Windows** (x64 and ARM) | [Installer](#windows), [ZIP](#windows) |
| **Debian, Ubuntu** (x64 and ARM) | [DEB package](#debian-and-ubuntu) |
| **Fedora** (x64 and ARM) | [RPM package](#fedora) |
| **Arch Linux** (x64) | [AUR](#arch-linux) |
| **Any Linux** | [Flatpak](#flatpak) (x64), [AppImage](#appimage), [archive](#other-distributions), [Nix](#nix), [source](#build-from-source) |

## macOS

One download for both Apple Silicon and Intel:

- [Download for Mac]({{ base }}/resonix-v{{ v }}-macos-universal.dmg)

Open the downloaded file and drag **Resonix** to **Applications**.

If you use [Homebrew](https://brew.sh), you can install it with:

```sh
brew install --cask crmne/tap/resonix
```

### First open on macOS

Open **Resonix** from Applications. If your Mac asks whether you want to
open an app downloaded from the internet, choose **Open**.

Starting with 0.8.0, the Mac download passes Apple's security checks. You do
not need to change security settings or run commands in Terminal to open it.

Quit the old app before opening Resonix. Your saved settings and sign-ins
carry over. If an older version reports that its update helper exited before
it was ready, install the DMG above manually once. The helper changes take
effect for updates started from 0.9.1 onward.

## Windows

Download and run the installer. It adds Resonix to the Start menu and does
not need an administrator password. Choose the first download for most PCs,
or the ARM version if your PC uses an ARM processor:

- [Windows installer (most PCs)]({{ base }}/resonix-v{{ v }}-x86_64-pc-windows-msvc{{ windows_installer }})
- [Windows installer (ARM PCs)]({{ base }}/resonix-v{{ v }}-aarch64-pc-windows-msvc{{ windows_installer }})
{% if windows_installer == ".msi" %}
The installer is a Windows Installer package (`.msi`). Resonix installed
with the earlier setup program (`-setup.exe`) moves to it when you run the new
installer, and keeps its settings, its folder and its Start menu entry.
Windows then lists one Resonix under installed apps. Versions 0.13.0 and
later also move to it through their own updates; 0.12.x and earlier no longer
find updates, so run the installer once by hand.
{% endif %}
To run Resonix without an installer, download a ZIP file, extract it, and
open `resonix.exe`. Since 0.13.4, it keeps its settings, state and caches
in a `resonix-data` folder next to it, so the folder carries everything
(see [portable copies](/settings-and-files/#portable-copies)).

- [Windows ZIP (most PCs)]({{ base }}/resonix-v{{ v }}-x86_64-pc-windows-msvc.zip)
- [Windows ZIP (ARM PCs)]({{ base }}/resonix-v{{ v }}-aarch64-pc-windows-msvc.zip)

The Windows executable and installer are signed as **PlentyLabs UG
(haftungsbeschränkt) & Co. KG**. SmartScreen may still show a warning while
the publisher builds reputation.

Resonix needs OpenGL 2.0 or newer, which every current Windows graphics
driver provides. In a virtual machine or remote session that only has the
Microsoft Basic Display Adapter, it cannot open its window and says so in a
message that links to this page; install the machine's graphics driver, or
enable GPU acceleration for the virtual machine.

To choose which app opens Spotify links, use **Settings → Apps → Default apps**
in Windows.

Uninstalling while Resonix runs removes nothing until it is closed. The
installer offers to close it for you; closing it that way ends it at once.

### Check a Windows download

To check a file against the [published checksums]({{ base }}/checksums.txt),
print its SHA-256 hash in PowerShell and compare it with that file's line,
ignoring letter case:

```powershell
Get-FileHash "$HOME\Downloads\resonix-v{{ v }}-*-pc-windows-msvc*"
```

If Windows Security flags a download, open **Windows Security → Virus &
threat protection → Protection history** and note the threat name. Include it,
and the file's hash, when you
[report the problem](https://github.com/ItsARCn/Resonix/issues/new?template=bug.yml).
To ask Microsoft to review the file, send it through
[Submit a file for malware analysis](https://www.microsoft.com/en-us/wdsi/filesubmission)
as incorrectly detected.

## Linux

The DEB, RPM, AppImage and archive need glibc 2.39 or newer: Ubuntu 24.04,
Debian 13, Fedora 41 or a newer release of each.

### Debian and Ubuntu

- [DEB package (Intel and AMD, 64-bit)]({{ base }}/resonix_{{ v }}_amd64.deb)
- [DEB package (ARM, 64-bit)]({{ base }}/resonix_{{ v }}_arm64.deb)

Install it with apt, which also installs the libraries it needs:

```sh
sudo apt install ~/Downloads/resonix_{{ v }}_amd64.deb   # or _arm64.deb
```

To update, download and install the new release the same way. When one is
out, Resonix's update button shows the command. To remove it, run
`sudo apt remove resonix`.

### Fedora

- [RPM package (Intel and AMD, 64-bit)]({{ base }}/resonix-{{ v }}-1.x86_64.rpm)
- [RPM package (ARM, 64-bit)]({{ base }}/resonix-{{ v }}-1.aarch64.rpm)

```sh
sudo dnf install ~/Downloads/resonix-{{ v }}-1.x86_64.rpm   # or .aarch64.rpm
```

To update, install the new release the same way. When one is out,
Resonix's update button shows the command, which dnf runs straight from
the release's address. To remove it, run `sudo dnf remove resonix`.

### Arch Linux

Resonix is in the AUR, Arch's community package collection. Installing it
also adds it to your app launcher. With [yay](https://github.com/Jguer/yay)
or [paru](https://github.com/Morganamilo/paru):

```sh
yay -S resonix-bin      # the released build, ready made
yay -S resonix          # the release, built from source
yay -S resonix-git      # built from the latest commit
```

`paru -S` takes the same package names.

### Flatpak

Download the [Resonix Flatpak]({{ base }}/resonix-v{{ v }}-x86_64.flatpak?flatpak-id=rocks.resonix.Resonix).
It requires Flatpak and the Freedesktop 25.08 runtime, a set of shared
components used by Flatpak apps:

```sh
flatpak install --user ~/Downloads/resonix-v{{ v }}-x86_64.flatpak
flatpak run rocks.resonix.Resonix
```

The application ID is **`rocks.resonix.Resonix`**.

To update this Flatpak installation, download and install the new release.
When one is out, Resonix's update button shows the command. Flathub
support is planned.

Packages from other stores are maintained by their publishers. Report
problems specific to those packages to their maintainers.

### AppImage

- [AppImage (Intel and AMD, 64-bit)]({{ base }}/resonix-{{ v }}-x86_64.AppImage)
- [AppImage (ARM, 64-bit)]({{ base }}/resonix-{{ v }}-aarch64.AppImage)

One file, no installation: make it executable and run it.

```sh
chmod +x ~/Downloads/resonix-{{ v }}-x86_64.AppImage
~/Downloads/resonix-{{ v }}-x86_64.AppImage
```

The AppImage bundles no libraries: like the DEB and RPM, it uses your
desktop's own libraries. Running it needs FUSE; without
FUSE, start it with `--appimage-extract-and-run`.

From the release after 0.13.3, the AppImage updates itself like the portable
downloads, as long as it sits in a folder of your own, such as
`~/Applications`: Resonix replaces the file in place and restarts it. It
also carries update information, so AppImageUpdate, AppImageLauncher, AM and
Gear Lever can update it too, downloading only what changed. Copies of 0.13.3
and earlier need one more manual download.

### Other distributions

- [Linux archive (Intel and AMD, 64-bit)]({{ base }}/resonix-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz)
- [Linux archive (ARM, 64-bit)]({{ base }}/resonix-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz)

Unpack, put `resonix` on your PATH, and copy the desktop entry and icon
from the bundled `packaging/` directory if you want it in your launcher and
handling `spotify:` links.
The binary needs the ALSA and PipeWire libraries, and Wayland or X11. It plays
through PipeWire, a PulseAudio server, or ALSA, whichever is running.

### Build from source

Download the [source archive]({{ base }}/resonix-v{{ v }}-source.tar.gz),
which the [download checksums]({{ base }}/checksums.txt) also cover, and
follow the [build instructions](/getting-started/#build-from-source).

## Nix

Add the repository [flake](https://github.com/ItsARCn/Resonix) to your
inputs:

```nix
inputs.resonix.url = "github:crmne/resonix";
```

On NixOS, install the default package:

```nix
environment.systemPackages = [
  inputs.resonix.packages."${pkgs.stdenv.hostPlatform.system}".default
];
```

### nix-darwin

On macOS, use the same `resonix` package to install Resonix as a Mac app:

```nix
environment.systemPackages = [
  inputs.resonix.packages."${pkgs.stdenv.hostPlatform.system}".resonix
];
environment.pathsToLink = [ "/Applications" ];
```

The bundle appears in `/Applications/Nix Apps`. With Home Manager,
`home.packages` is enough; its darwin support links app bundles into
`~/Applications`:

```nix
home.packages = [
  inputs.resonix.packages."${pkgs.stdenv.hostPlatform.system}".resonix
];
```
