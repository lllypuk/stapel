# Образ CI: toolchain из rust-toolchain.toml, инструменты make check/cov/mutants, lavapipe, alsa для cpal.
# Тег — CI_IMAGE в .gitlab-ci.yml; поднимается вместе с правкой этого файла.
FROM rust:1.99.0-trixie

RUN apt-get update \
    && apt-get install -y --no-install-recommends libvulkan1 mesa-vulkan-drivers libasound2-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY rust-toolchain.toml /tmp/toolchain/
RUN cd /tmp/toolchain && rustup toolchain install && rustup show && rm -rf /tmp/toolchain

ARG BINSTALL=1.25.3
RUN curl -fsSL "https://github.com/cargo-bins/cargo-binstall/releases/download/v${BINSTALL}/cargo-binstall-x86_64-unknown-linux-musl.tgz" \
        | tar -xz -C /usr/local/cargo/bin \
    && cargo binstall --no-confirm --locked \
        cargo-llvm-cov@0.9.1 cargo-mutants@27.1.0 cargo-deny@0.20.2 cargo-machete@0.9.2 typos-cli@1.51.1 \
    && rm -rf /usr/local/cargo/registry
