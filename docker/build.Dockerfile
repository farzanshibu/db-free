# SOT: linux-bundle-image, docker-build-runner
#
# WHAT:  The Linux bundling environment, pinned to the same Ubuntu the release
#        workflow's ubuntu-22.04 runner uses.
# WHY:   webkit2gtk, libclang (librocksdb-sys' bindgen) and cmake (aws-lc-sys)
#        are painful to install on a developer box and impossible without root.
#        A container has root, so the build stops depending on the host.
# HOW:   docker compose -f docker-compose.build.yml run --rm linux-bundle
# WHERE: .github/workflows/release.yml (the apt list this mirrors)
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

# The release workflow's apt line, plus what GitHub's runner image already has
# (curl, git, a compiler) and the actions would otherwise provide.
RUN apt-get update && apt-get install -y --no-install-recommends \
      libwebkit2gtk-4.1-dev \
      libappindicator3-dev \
      librsvg2-dev \
      patchelf \
      libdbus-1-dev \
      pkg-config \
      libclang-dev \
      clang \
      cmake \
      build-essential \
      ca-certificates \
      curl \
      file \
      git \
      xz-utils \
      desktop-file-utils \
      fuse3 \
      libfuse2 \
 && rm -rf /var/lib/apt/lists/*

# librocksdb-sys runs bindgen at build time; it loads libclang by path.
ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib
RUN test -e "$LIBCLANG_PATH/libclang.so.1" || { echo "libclang not at $LIBCLANG_PATH"; exit 1; }

ARG NODE_MAJOR=22
RUN curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - \
 && apt-get install -y --no-install-recommends nodejs \
 && rm -rf /var/lib/apt/lists/*

ARG PNPM_VERSION=10
RUN corepack enable && corepack prepare "pnpm@${PNPM_VERSION}" --activate

ARG RUST_TOOLCHAIN=stable
ENV CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup PATH=/usr/local/cargo/bin:$PATH
# clippy comes with the toolchain because `pnpm check` runs it; the minimal
# profile omits it, and this image is meant to be able to run what CI runs.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain "$RUST_TOOLCHAIN" --component clippy \
 && rustc --version && cargo --version && cargo clippy --version

WORKDIR /work
