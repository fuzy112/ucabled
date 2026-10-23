# Build environment for the Debian packages, used by `nix run .#deb`.
#
# Debian trixie is the oldest distribution the packaging targets that has
# gtk4-layer-shell 1.0; its rustc is older than Cargo.toml's MSRV, so start
# from the matching upstream Rust image instead of the distribution one.
# The flake app passes Cargo.toml's rust-version as RUST_VERSION.
ARG RUST_VERSION=1.92
FROM docker.io/library/rust:${RUST_VERSION}-trixie

# A rootless podman without subordinate UID ranges maps only the invoking
# user.  apt normally drops to _apt for downloads, which is unmapped then;
# keep it as root inside the container.
RUN printf '%s\n' 'APT::Sandbox::User "root";' > /etc/apt/apt.conf.d/99rootless

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        build-essential \
        debhelper \
        dpkg-dev \
        fakeroot \
        gettext \
        pkgconf \
        libdbus-1-dev \
        libgtk-4-dev \
        libgtk4-layer-shell-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
