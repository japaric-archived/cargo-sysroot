#!/bin/bash

set -ex

main() {
  setup

  test_supported_target
  test_custom_target
  # Enable this test when rust-lang/cargo#2241
  #test_cargo_project_with_deps
}

die() {
  echo "$@" 1>&2
  exit 1
}

setup() {
  cargo install --path .
}

test_supported_target() {
  local triple=arm-unknown-linux-gnueabihf

  # build sysroot in debug mode
  cargo sysroot --target $triple sysroot --verbose

  # check that libcore was built in debug mode
  [ -e sysroot/debug/lib/rustlib/$triple/lib/libcore.rlib ] || die

  # build sysroot in release mode
  cargo sysroot --target $triple sysroot --release --verbose

  # check that libcore was built in release mode
  [ -e sysroot/release/lib/rustlib/$triple/lib/libcore.rlib ] || die

  # check that the debug mode libcore still exists
  [ -e sysroot/debug/lib/rustlib/$triple/lib/libcore.rlib ] || die

  # clean up
  rm -r sysroot
}

test_custom_target() {
  cat >custom.json <<EOF
{
  "arch": "arm",
  "llvm-target": "thumbv7m-none-eabi",
  "os": "none",
  "target-endian": "little",
  "target-pointer-width": "32"
}
EOF

  # test --target triple
  cargo sysroot --target custom sysroot --verbose

  # confirm existence of build artifacts
  [ -e sysroot/debug/lib/rustlib/custom/lib/libcore.rlib ] || die

  # clean up
  rm -r sysroot

  # test --target path/to/triple.json
  cp custom.json ..
  cargo sysroot --target ../custom.json sysroot --verbose

  # confirm existence of build artifacts
  [ -e sysroot/debug/lib/rustlib/custom/lib/libcore.rlib ] || die

  # check that the original spec file is still there
  [ -e ../custom.json ] || die

  # check that the copied spec file was removed
  [ -e sysroot/src/libcore/custom.json ] && die

  # clean up
  rm ../custom.json
  rm -r sysroot
}

test_cargo_project_with_deps() {
  local triple=arm-unknown-linux-gnueabif

  cargo new foo

  pushd foo
  echo 'spin = "0.3.5"' >> Cargo.toml

  cat >src/lib.rs <<EOF
#![no_std]
extern crate spin;
EOF

  cargo sysroot --target $triple target/sysroot

  RUSTFLAGS='--sysroot target/sysroot/debug' cargo build --target $triple
}

main
