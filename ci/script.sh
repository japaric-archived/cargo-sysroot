# `script` phase: you usually build, test and generate docs in this phase

set -ex

case "$TRAVIS_OS_NAME" in
  linux)
    host=x86_64-unknown-linux-gnu
    ;;
  osx)
    host=x86_64-apple-darwin
    ;;
esac

# NOTE Workaround for rust-lang/rust#31907 - disable doc tests when cross compiling
# This has been fixed in the nightly channel but it would take a while to reach the other channels
if [ "$host" != "$TARGET" ] && [ "$CHANNEL" != "nightly" ]; then
  if [ "$TRAVIS_OS_NAME" = "osx" ]; then
    brew install gnu-sed --default-names
  fi

  find src -name '*.rs' -type f | xargs sed -i -e 's:\(//.\s*```\):\1 ignore,:g'
fi

cargo build --target $TARGET --verbose

if [ "$CHANNEL" = "nightly" ]; then
  cargo test --target $TARGET
fi
