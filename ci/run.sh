#!/bin/bash

set -ex

cargo install --path .

# Test with supported target
cargo sysroot --target arm-unknown-linux-gnueabihf sysroot --verbose
tree sysroot/lib
rm -r sysroot

# Test with custom target
cat >custom.json <<EOF
{
  "arch": "arm",
  "llvm-target": "thumbv7m-none-eabi",
  "os": "none",
  "target-endian": "little",
  "target-pointer-width": "32"
}
EOF
cp custom.json ..
cargo sysroot --target ../custom.json sysroot --verbose
tree sysroot/lib
rm -r sysroot
rm custom.json
