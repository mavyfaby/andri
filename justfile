# andri dev workflows. Run `just` to list recipes.
# Install just: https://github.com/casey/just  (brew install just)

# Default: show available recipes
default:
    @just --list

# Format, lint (deny warnings), and test — the pre-commit gate (matches CI).
check: fmt-check clippy test

# Run the full test suite.
test:
    cargo test --all

# Check formatting without modifying files (CI uses this).
fmt-check:
    cargo fmt --all -- --check

# Apply formatting.
fmt:
    cargo fmt --all

# Lint, treating warnings as errors (matches CI).
clippy:
    cargo clippy --all-targets -- -D warnings

# Debug build.
build:
    cargo build

# Optimized release build (the tuned profile).
release:
    cargo build --release

# Run a server (receiver). Example: just server, or: just server "--format gbps"
server *ARGS:
    cargo run --release -- --server {{ARGS}}

# Run a client against a host. Example: just client 127.0.0.1 "--tcp -d 5"
client HOST *ARGS:
    cargo run --release -- --client {{HOST}} {{ARGS}}

# Loopback smoke test: start a server, run a short test of each mode, stop it.
# Proves the whole pipeline end-to-end (the integration coverage we don't unit-test).
smoke: release
    #!/usr/bin/env bash
    set -euo pipefail
    pkill -f 'target/release/andri' 2>/dev/null || true
    sleep 0.3
    ./target/release/andri --server >/tmp/andri-smoke.log 2>&1 &
    SRV=$!
    trap "kill $SRV 2>/dev/null || true" EXIT
    sleep 0.5
    echo "── TCP ──"
    ./target/release/andri --client 127.0.0.1 --tcp -d 2 --format gbps
    echo "── UDP ──"
    ./target/release/andri --client 127.0.0.1 --udp --bitrate 500M -d 2
    echo "── FILE ──"
    head -c 33554432 /dev/urandom > /tmp/andri-smoke.bin
    ./target/release/andri --client 127.0.0.1 --file /tmp/andri-smoke.bin
    rm -f /tmp/andri-smoke.bin

# Scripted demo for screen-recording. Paced with pauses and narration so the
# disk-vs-network story reads clearly on screen. Run it while recording (e.g.
# with Screen Studio) to capture a clean take.
demo: release
    #!/usr/bin/env bash
    set -euo pipefail
    pkill -f 'target/release/andri' 2>/dev/null || true
    sleep 0.3
    ./target/release/andri --server >/tmp/andri-demo.log 2>&1 &
    SRV=$!
    trap "kill $SRV 2>/dev/null || true; rm -f /tmp/andri-demo.bin" EXIT
    sleep 0.5
    say() { printf '\n\033[1;36m# %s\033[0m\n' "$1"; sleep 2; }

    say "andri — one tool, three measurements. First, raw TCP throughput:"
    ./target/release/andri --client 127.0.0.1 --tcp -d 3 --format gbps
    sleep 2

    say "UDP — same link, but now with packet loss and jitter:"
    ./target/release/andri --client 127.0.0.1 --udp --bitrate 1G -d 3 --format gbps
    sleep 2

    say "Now the differentiator. A real file transfer — includes disk read:"
    head -c 2147483648 /dev/urandom > /tmp/andri-demo.bin
    ./target/release/andri --client 127.0.0.1 --file /tmp/andri-demo.bin --format gbps
    sleep 2

    say "Same file, --null-source: skip the disk, measure the network alone."
    say "The gap between these two = your disk, not your wire."
    ./target/release/andri --client 127.0.0.1 --file /tmp/andri-demo.bin --null-source --format gbps
    sleep 2

# Verify the crate packages cleanly for crates.io (no upload).
publish-dry:
    cargo publish --dry-run

# Dry-run a release: tag a pre-release (builds all platform binaries in CI,
# SKIPS crates.io publish). Example: just tag-test 0.0.1
[confirm("Push pre-release tag and trigger CI builds? [y/N]")]
tag-test VERSION:
    git tag -a v{{VERSION}}-test -m "dry-run release test"
    git push origin v{{VERSION}}-test
    @echo "Watch the build at: https://github.com/mavyfaby/andri/actions"

# Cut a real release: tag vX.Y.Z (CI publishes to crates.io + builds binaries).
# Guards: version must match Cargo.toml, tag must not already exist, and the new
# version must sort strictly above the latest release tag. crates.io publishes
# are PERMANENT — this cannot be undone. Example: just tag 0.1.0
[confirm("Cut a REAL release? This publishes to crates.io PERMANENTLY. [y/N]")]
tag VERSION:
    #!/usr/bin/env bash
    set -euo pipefail
    new="{{VERSION}}"

    # 1. Must match the version declared in Cargo.toml.
    if ! grep -q "^version = \"$new\"" Cargo.toml; then
        echo "error: Cargo.toml version != $new (bump Cargo.toml first)" >&2
        exit 1
    fi

    # 2. Tag must not already exist (locally or on the remote).
    git fetch --tags --quiet
    if git rev-parse -q --verify "refs/tags/v$new" >/dev/null; then
        echo "error: tag v$new already exists" >&2
        exit 1
    fi

    # 3. New version must be strictly greater than the latest release tag.
    #    Consider only real releases (vX.Y.Z), not pre-release/-test tags.
    latest=$(git tag --list 'v[0-9]*' \
        | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
        | sed 's/^v//' | sort -V | tail -1 || true)
    if [ -n "$latest" ]; then
        highest=$(printf '%s\n%s\n' "$latest" "$new" | sort -V | tail -1)
        if [ "$new" = "$latest" ] || [ "$highest" != "$new" ]; then
            echo "error: v$new is not greater than latest release v$latest" >&2
            exit 1
        fi
        echo "latest release: v$latest  →  new: v$new"
    else
        echo "first release: v$new"
    fi

    git tag -a "v$new" -m "andri v$new"
    git push origin "v$new"
    echo "Release running at: https://github.com/mavyfaby/andri/actions"

# Clean build artifacts.
clean:
    cargo clean
