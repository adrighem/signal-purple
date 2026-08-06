ARG CA_IMAGE=docker.io/library/alpine:3.23.3@sha256:59855d3dceb3ae53991193bd03301e082b2a7faa56a514b03527ae0ec2ce3a95
ARG BASE_IMAGE=debian:13-slim@sha256:9bb8a3626890e084ab54e888fdd7c4b6d2f119071cd4c5dc5fecb4d73062aa5f
FROM ${CA_IMAGE} AS ca-bootstrap
FROM ${BASE_IMAGE}

ARG DISTRO_ID=debian-13
ARG DEBIAN_SNAPSHOT=20260722T000000Z
ARG UBUNTU_SNAPSHOT=20260805T000000Z
ARG RUST_ARCHIVE=rust-1.95.0-x86_64-unknown-linux-gnu.tar.gz
ARG RUST_ARCHIVE_SHA256=a47ac940abd12399d59ad15c877e7113fa35f2b9ec7e6a8a045d4fd8b9741dea

COPY --from=ca-bootstrap \
    /etc/ssl/certs/ca-certificates.crt \
    /etc/ssl/certs/ca-certificates.crt

RUN set -eu; \
    test -s /etc/ssl/certs/ca-certificates.crt; \
    printf '%s\n' \
        'Acquire::https::CaInfo "/etc/ssl/certs/ca-certificates.crt";' \
        'APT::Update::Error-Mode "any";' \
        > /etc/apt/apt.conf.d/99release-ca; \
    . /etc/os-release; \
    case "$DISTRO_ID" in \
        debian-13) \
            test "$ID" = debian; \
            test "$VERSION_ID" = 13; \
            mv /etc/apt/sources.list.d/debian.sources \
                /etc/apt/debian.sources.disabled; \
            printf '%s\n' \
                'Types: deb' \
                "URIs: https://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}/" \
                'Suites: trixie' \
                'Components: main' \
                'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
                'Check-Valid-Until: no' \
                '' \
                'Types: deb' \
                "URIs: https://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}/" \
                'Suites: trixie-security' \
                'Components: main' \
                'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
                'Check-Valid-Until: no' \
                > /etc/apt/sources.list.d/release-snapshot.sources; \
            ;; \
        ubuntu-24.04-lts) \
            test "$ID" = ubuntu; \
            test "$VERSION_ID" = 24.04; \
            rm -f /etc/apt/sources.list.d/ubuntu.sources; \
            printf '%s\n' \
                'Types: deb' \
                "URIs: https://snapshot.ubuntu.com/ubuntu/${UBUNTU_SNAPSHOT}/" \
                'Suites: noble noble-updates' \
                'Components: main universe' \
                'Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg' \
                'Check-Valid-Until: no' \
                '' \
                'Types: deb' \
                "URIs: https://snapshot.ubuntu.com/ubuntu/${UBUNTU_SNAPSHOT}/" \
                'Suites: noble-security' \
                'Components: main universe' \
                'Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg' \
                'Check-Valid-Until: no' \
                > /etc/apt/sources.list.d/release-snapshot.sources; \
            ;; \
        *) \
            printf 'unsupported release distribution: %s\n' "$DISTRO_ID" >&2; \
            exit 1; \
            ;; \
    esac; \
    apt-get update; \
    DEBIAN_FRONTEND=noninteractive \
        apt-get install --yes --no-install-recommends \
        build-essential ca-certificates clang cmake curl debhelper dpkg-dev \
        fakeroot git gzip libclang-dev libgdk-pixbuf-2.0-dev \
        libglib2.0-dev libpurple-dev libsecret-1-dev libssl-dev ninja-build \
        pkgconf protobuf-compiler python3 rpm tar util-linux xz-utils

RUN set -eu; \
    archive="/tmp/${RUST_ARCHIVE}"; \
    source_dir="/tmp/${RUST_ARCHIVE%.tar.gz}"; \
    attempt=0; \
    until curl --fail --location --proto '=https' --tlsv1.2 \
            --continue-at - --output "$archive" \
            "https://static.rust-lang.org/dist/${RUST_ARCHIVE}"; do \
        attempt=$((attempt + 1)); \
        if [ "$attempt" -ge 20 ]; then exit 1; fi; \
        sleep 5; \
    done; \
    printf '%s  %s\n' "$RUST_ARCHIVE_SHA256" "$archive" \
        | sha256sum --check; \
    tar -xzf "$archive" -C /tmp; \
    "$source_dir/install.sh" --prefix=/usr/local --without=rust-docs; \
    rustc --version --verbose; \
    cargo --version --verbose; \
    rm -rf "$archive" "$source_dir"

ENV LC_ALL=C.UTF-8
ENV TZ=UTC

WORKDIR /workspace
