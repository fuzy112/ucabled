{ pkgs }:

let
  # NixOS only installs /etc/containers/policy.json when
  # virtualisation.containers is enabled.  The app invokes the podman
  # package directly, so point it at a policy of its own; the builder pulls
  # public images by digest and does not rely on image signatures.
  policy = pkgs.writeText "ucabled-deb-policy.json" ''
    {
      "default": [
        { "type": "insecureAcceptAnything" }
      ]
    }
  '';
in
pkgs.writeShellApplication {
  name = "ucabled-deb";
  runtimeInputs = with pkgs; [
    coreutils
    gawk
    git
    gnugrep
    gnused
    podman
  ];
  text = ''
    usage() {
      cat <<'EOF'
    Usage: nix run .#deb [-- [--help] [DISTRO ...]]

    Build the three Debian packages in podman containers, one image per
    distribution.  Each DISTRO is debian13 or ubuntu26.04 ("all" means
    both); with no argument both are built.  Artifacts land in
    <out>/<distro>/.  A development build (a dirty tree, or a checkout
    that is not a clean release tag) gets the commit date and short hash
    in its version; a release tarball does not.

    It runs rootless by default; with sudo it uses a rootful build.  The
    container images are built on first use and cached by podman.  The
    cargo registry is kept in a shared named volume; the target directory
    is per distribution, because objects linked against one
    distribution's glibc must not be reused for another.

    Environment:
      UCABLED_DEB_OUT    output directory (default: <repo>/debian-build)
      UCABLED_DEB_IMAGE  container image name (default:
                         localhost/ucabled-deb-builder)
    EOF
    }

    distros=()
    for arg in "$@"; do
      case "$arg" in
        -h | --help)
          usage
          exit 0
          ;;
        all)
          distros=(debian13 ubuntu26.04)
          ;;
        debian13 | ubuntu26.04)
          distros+=("$arg")
          ;;
        *)
          echo "ucabled-deb: unexpected argument: $arg" >&2
          usage >&2
          exit 2
          ;;
      esac
    done
    if [ "''${#distros[@]}" -eq 0 ]; then
      distros=(debian13 ubuntu26.04)
    fi

    repo=$(git rev-parse --show-toplevel 2>/dev/null || true)
    if [ -z "$repo" ] || [ ! -f "$repo/debian/control" ]; then
      # sudo makes git refuse a repository owned by another user; find the
      # source tree from the working directory instead.
      repo=$(pwd)
      while [ "$repo" != "/" ] && [ ! -f "$repo/debian/control" ]; do
        repo=$(dirname "$repo")
      done
    fi
    if [ ! -f "$repo/debian/control" ] || [ ! -f "$repo/Cargo.toml" ]; then
      echo "ucabled-deb: run this from the ucabled source tree" >&2
      exit 1
    fi

    if [ "$(id -u)" -ne 0 ] &&
      { ! grep -q "^$(id -un):" /etc/subuid 2>/dev/null ||
        ! grep -q "^$(id -un):" /etc/subgid 2>/dev/null; }; then
      cat >&2 <<EOF
    ucabled-deb: rootless podman needs a subordinate UID/GID range.
    On NixOS add:
      users.users.$(id -un).subUidRanges = [{ startUid = 100000; count = 65536; }];
      users.users.$(id -un).subGidRanges = [{ startGid = 100000; count = 65536; }];
    then run: sudo nixos-rebuild switch
    Alternatively build inside a NixOS VM with the same ranges configured.
    Or run this app with sudo for a rootful build.
    EOF
      exit 1
    fi

    # Keep the container toolchain in step with the MSRV the Debian packaging
    # enforces.  The tag exists for every released Rust minor.
    rust_version=$(sed -n 's/^rust-version = "\(.*\)"/\1/p' "$repo/Cargo.toml")
    rust_version=''${rust_version:-1.92}

    image_name="''${UCABLED_DEB_IMAGE:-localhost/ucabled-deb-builder}"
    out="''${UCABLED_DEB_OUT:-$repo/debian-build}"

    # NixOS only writes /etc/containers/policy.json when
    # virtualisation.containers is enabled.  Keep podman's config and image
    # store in a dedicated state directory instead of relying on that module.
    if [ "$(id -u)" -eq 0 ]; then
      podman_state="/var/lib/ucabled-deb/podman"
      podman_runroot="/run/ucabled-deb-podman"
    else
      podman_state="''${XDG_DATA_HOME:-$HOME/.local/share}/ucabled-deb/podman"
      podman_runroot="''${XDG_RUNTIME_DIR:-/tmp}/ucabled-deb-podman"
    fi
    podman_config="$podman_state/config"
    podman_root="$podman_state/storage"
    mkdir -p "$podman_config/.config/containers" "$podman_runroot"
    # install, not cp: the store copy is read-only, and plain cp cannot
    # overwrite the read-only destination from a previous run.
    install -m 0644 ${policy} "$podman_config/.config/containers/policy.json"

    run_podman() {
      HOME="$podman_config" XDG_CONFIG_HOME="$podman_config/.config" \
        podman --root "$podman_root" --runroot "$podman_runroot" "$@"
    }

    # A release tarball has no .git, and a clean checkout at a release tag
    # is a release build; both keep the changelog version.  Every other
    # tree gets the commit date and short hash, computed here because the
    # container's copy of the tree has no .git.
    dev_version=$(cd "$repo" && debian/dev-version.sh --print)

    mkdir -p "$out"

    for distro in "''${distros[@]}"; do
      case "$distro" in
        debian13)
          containerfile=${./debian-builder.Containerfile}
          image="$image_name:$rust_version-trixie"
          ;;
        ubuntu26.04)
          containerfile=${./debian-builder-ubuntu.Containerfile}
          image="$image_name:$rust_version-ubuntu26.04"
          ;;
      esac
      outdir="$out/$distro"
      mkdir -p "$outdir"
      rm -f "$outdir"/*.deb "$outdir"/*.buildinfo "$outdir"/*.changes

      run_podman --storage-driver=vfs build \
        --pull=missing \
        --network=host \
        --build-arg "RUST_VERSION=$rust_version" \
        -t "$image" \
        -f "$containerfile" \
        "$repo/nix"

      run_podman --storage-driver=vfs run \
        --rm \
        --network=host \
        -v "$repo:/src:ro" \
        -v "$outdir:/out" \
        -v ucabled-deb-cargo:/cache/cargo \
        -v "ucabled-deb-target-$distro:/cache/target" \
        -e CARGO=/usr/local/cargo/bin/cargo \
        -e CARGO_HOME=/cache/cargo \
        -e CARGO_TERM_COLOR=never \
        -e "UCABLED_DEV_VERSION=$dev_version" \
        -w /src \
        "$image" \
        bash -c '
          set -euo pipefail

          # Build from a copy so dpkg-buildpackage can write to the parent
          # directory without dropping artifacts into the host checkout, and
          # so a clean target cannot remove the cargo cache.
          rm -rf /tmp/ucabled-deb
          mkdir -p /tmp/ucabled-deb
          tar -C /src \
            --exclude=.git \
            --exclude=target \
            --exclude=result \
            --exclude=site \
            --exclude=debian-build \
            -cf - . | tar -C /tmp/ucabled-deb -xf -

          cd /tmp/ucabled-deb
          debian/rules clean
          ln -s /cache/target target
          # dpkg-buildpackage reads the version before debian/rules runs;
          # the suffix was computed on the host, where .git still exists.
          debian/dev-version.sh

          # -nc: the clean above ran before the target symlink was installed.
          # -d: cargo and rustc come from the toolchain in the image, not
          #     from apt, so the Build-Depends version check would reject the
          #     distribution ones.
          dpkg-buildpackage -nc -b -us -uc -d

          cp -v /tmp/*.deb /tmp/*.buildinfo /tmp/*.changes /out/
        '
    done

    if [ "$(id -u)" -eq 0 ] && [ -n "''${SUDO_UID:-}" ]; then
      chown -R "''${SUDO_UID}:''${SUDO_GID:-$(id -g "$SUDO_UID")}" "$out"
    fi

    echo
    echo "Packages written to $out:"
    ls -lR "$out"
  '';
}
