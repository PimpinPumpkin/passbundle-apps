# Builds the PassBundle apps for Linux, and runs them under a virtual display
# to measure size, memory, and startup (tools/linux/measure.sh).
FROM ubuntu:24.04@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac

ARG FLUTTER_VERSION=3.47.6
ARG RUST_VERSION=1.99.0
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git unzip xz-utils zip file \
      build-essential clang cmake ninja-build pkg-config \
      libgtk-3-dev liblzma-dev libglu1-mesa libgl1-mesa-dri \
      xvfb xauth xdotool dbus-x11 procps time \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m builder
USER builder
WORKDIR /home/builder

# Rust, at the same version as the server and the desktop agent.
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION}
ENV PATH=/home/builder/.cargo/bin:/home/builder/flutter/bin:$PATH

# Flutter, at a stable release (its tag; the commit is checked below).
RUN git clone --depth 1 --branch ${FLUTTER_VERSION} https://github.com/flutter/flutter.git \
    && test "$(git -C flutter rev-parse HEAD)" = "5fc346839b5d0eef006ed8404392afb4dfae428d" \
    && flutter config --no-analytics --enable-linux-desktop \
    && flutter precache --linux \
    && flutter --version

# Screenshots of the virtual display, for tools/linux/measure.sh.
USER root
RUN apt-get update && apt-get install -y --no-install-recommends x11-apps imagemagick \
    && rm -rf /var/lib/apt/lists/*
USER builder
