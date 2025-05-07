#!/bin/sh -e

echo "TEST: build for ARM target"
cargo build --target=thumbv7em-none-eabihf -p dot15d4-frame3

echo "TEST: no features"
cargo test -p dot15d4-frame3 --no-default-features

echo "TEST: security"
cargo test -p dot15d4-frame3 --no-default-features --features=security

echo "TEST: tsch"
cargo test -p dot15d4-frame3 --no-default-features --features=tsch

echo "TEST: security,tsch"
cargo test -p dot15d4-frame3 --no-default-features --features=security,tsch
