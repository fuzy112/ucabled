# Build environment for the Ubuntu packages, used by
# `nix run .#deb -- ubuntu26.04`.
#
# Ubuntu 24.04 LTS predates gtk4-layer-shell (first packaged in 25.04), so
# 26.04 LTS is the oldest Ubuntu the packaging targets.  Its rustc is older
# than Cargo.toml's MSRV and the upstream rust image has no Ubuntu variant,
# so the toolchain comes from rustup instead.  The flake app passes
# Cargo.toml's rust-version as RUST_VERSION.
ARG RUST_VERSION=1.92
FROM docker.io/library/ubuntu:26.04
ARG RUST_VERSION

# A rootless podman without subordinate UID ranges maps only the invoking
# user.  apt normally drops to _apt for downloads, which is unmapped then;
# keep it as root inside the container.
RUN printf '%s\n' 'APT::Sandbox::User "root";' > /etc/apt/apt.conf.d/99rootless

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        debhelper \
        dpkg-dev \
        fakeroot \
        gettext \
        pkgconf \
        libdbus-1-dev \
        libgtk-4-dev \
        libgtk4-layer-shell-dev \
 && rm -rf /var/lib/apt/lists/*

# Match the upstream rust image layout (/usr/local/cargo/bin/cargo) so the
# flake app can point CARGO at the same path for both distributions.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION"

WORKDIR /src
