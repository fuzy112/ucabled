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
    git
    gnugrep
    gnused
    podman
  ];
  text = ''
    usage() {
      cat <<'EOF'
    Usage: nix run .#deb [-- --help]

    Build the three Debian packages in a podman container.  It runs
    rootless by default; with sudo it uses a rootful build.  The container
    image is built on first use and cached by podman; the cargo registry
    and target directory are kept in named volumes.

    Environment:
      UCABLED_DEB_OUT    output directory (default: <repo>/debian-build)
      UCABLED_DEB_IMAGE  container image tag
    EOF
    }

    case "''${1:-}" in
      -h | --help)
        usage
        exit 0
        ;;
      "")
        ;;
      *)
        echo "ucabled-deb: unexpected argument: $1" >&2
        usage >&2
        exit 2
        ;;
    esac

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

    image="''${UCABLED_DEB_IMAGE:-localhost/ucabled-deb-builder:$rust_version-trixie}"
    out="''${UCABLED_DEB_OUT:-$repo/debian-build}"
    containerfile=${./debian-builder.Containerfile}

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
    cp ${policy} "$podman_config/.config/containers/policy.json"

    run_podman() {
      HOME="$podman_config" XDG_CONFIG_HOME="$podman_config/.config" \
        podman --root "$podman_root" --runroot "$podman_runroot" "$@"
    }

    mkdir -p "$out"
    rm -f "$out"/*.deb "$out"/*.buildinfo "$out"/*.changes

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
      -v "$out:/out" \
      -v ucabled-deb-cargo:/cache/cargo \
      -v ucabled-deb-target:/cache/target \
      -e CARGO=/usr/local/cargo/bin/cargo \
      -e CARGO_HOME=/cache/cargo \
      -e CARGO_TERM_COLOR=never \
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

        # -nc: the clean above ran before the target symlink was installed.
        # -d: cargo and rustc come from the Rust image, not from apt, so the
        #     Build-Depends version check would reject the distribution ones.
        dpkg-buildpackage -nc -b -us -uc -d

        cp -v /tmp/*.deb /tmp/*.buildinfo /tmp/*.changes /out/
      '

    if [ "$(id -u)" -eq 0 ] && [ -n "''${SUDO_UID:-}" ]; then
      chown -R "''${SUDO_UID}:''${SUDO_GID:-$(id -g "$SUDO_UID")}" "$out"
    fi

    echo
    echo "Packages written to $out:"
    ls -l "$out"
  '';
}
