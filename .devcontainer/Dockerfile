# pico-gs2 development container.
#
# Everything needed to build and verify the design:
#   * sinter (pinned release binary) compiles hw/*.snt to SystemVerilog and a
#     cycle-accurate Rust simulator.
#   * OSS CAD Suite: Yosys, nextpnr-ecp5, ecppack, ecppll, Verilator,
#     openFPGALoader. The same release Sinter's own CI tests against.
#   * Rust (stable) for the testbenches and chip models in sim/ and models/.
#   * just, the task runner (see justfile + scripts/).
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        libtcl8.6 \
        locales \
        openssh-client \
        pkg-config \
        python3 \
        sudo \
        usbutils \
        xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && locale-gen en_US.UTF-8

# --- OSS CAD Suite (Yosys, nextpnr-ecp5, ecppack, ecppll, Verilator, ...) ---
ARG OSS_CAD_SUITE_DATE=2026-01-27
ARG OSS_CAD_SUITE_VERSION=20260127
ENV OSS_CAD_SUITE_PATH=/opt/oss-cad-suite
RUN curl -fsSL \
        "https://github.com/YosysHQ/oss-cad-suite-build/releases/download/${OSS_CAD_SUITE_DATE}/oss-cad-suite-linux-x64-${OSS_CAD_SUITE_VERSION}.tgz" \
        -o /tmp/oss-cad-suite.tgz \
    && tar -xzf /tmp/oss-cad-suite.tgz -C /opt \
    && rm /tmp/oss-cad-suite.tgz
ENV PATH="${OSS_CAD_SUITE_PATH}/bin:${PATH}"

# --- just (task runner) ------------------------------------------------------
ARG JUST_VERSION=1.54.0
RUN curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh \
        | bash -s -- --tag "${JUST_VERSION}" --to /usr/local/bin \
    && just --version

# --- sinter (pinned static binary, checksum-verified) ------------------------
ARG SINTER_VERSION=v1.0.0-rc.1
RUN cd /tmp \
    && base="https://github.com/sinter-lang/sinter-lang/releases/download/${SINTER_VERSION}" \
    && asset="sinter-${SINTER_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    && curl -fsSLO "${base}/${asset}" && curl -fsSLO "${base}/${asset}.sha256" \
    && sha256sum -c "${asset}.sha256" \
    && tar -xzf "${asset}" -C /usr/local/bin sinter \
    && rm "${asset}" "${asset}.sha256" \
    && sinter --version

# --- Non-root user -----------------------------------------------------------
# Ubuntu 24.04 ships a uid-1000 "ubuntu" user; replace it so vscode gets uid
# 1000 and bind-mounted files keep the host owner.
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID
RUN if getent passwd $USER_UID >/dev/null; then \
        userdel -r "$(getent passwd $USER_UID | cut -d: -f1)"; \
    fi \
    && groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m -s /bin/bash $USERNAME \
    && usermod -aG plugdev,dialout $USERNAME \
    && echo "$USERNAME ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

USER $USERNAME
ENV HOME=/home/$USERNAME

# --- Rust via rustup (stable) ------------------------------------------------
ENV RUSTUP_HOME=$HOME/.rustup \
    CARGO_HOME=$HOME/.cargo \
    PATH=$HOME/.cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain stable \
            --component rustfmt --component clippy \
    && rustc --version

# --- Claude Code CLI (native installer, self-updating) -----------------------
ENV PATH=$HOME/.local/bin:$PATH
RUN curl -fsSL https://claude.ai/install.sh | bash \
    && claude --version

# Pre-create the cargo registry tree so the devcontainer's named volume
# (mounted at ~/.cargo/registry) is created owned by vscode, not root.
RUN mkdir -p $HOME/.cargo/registry/cache \
        $HOME/.cargo/registry/index \
        $HOME/.cargo/registry/src

# Persist tool PATHs for login shells.
RUN echo 'export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/oss-cad-suite/bin:$PATH"' >> $HOME/.bashrc \
    && echo 'export OSS_CAD_SUITE_PATH=/opt/oss-cad-suite' >> $HOME/.bashrc

ENV LANG=en_US.UTF-8
