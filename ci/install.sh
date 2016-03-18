# `install` phase: install stuff needed for the `script` phase

set -ex

# Install multirust
git clone https://github.com/brson/multirust
pushd multirust
./build.sh
./install.sh --prefix=~/multirust
multirust default $CHANNEL
rustc -V
cargo -V
popd

case "$TRAVIS_OS_NAME" in
  linux)
    host=x86_64-unknown-linux-gnu
    ;;
  osx)
    host=x86_64-apple-darwin
    ;;
esac

# Install standard libraries needed for cross compilation
if [ "$host" != "$TARGET" ]; then
  if [ "$CHANNEL" = "nightly" ]; then
    multirust add-target nightly $TARGET
  else
    if [ "$CHANNEL" = "stable" ]; then
      # e.g. 1.6.0
      version=$(rustc -V | cut -d' ' -f2)
    else
      version=beta
    fi

    tarball=rust-std-${version}-${TARGET}

    curl -Os http://static.rust-lang.org/dist/${tarball}.tar.gz

    tar xzf ${tarball}.tar.gz

    ${tarball}/install.sh --prefix=$(rustc --print sysroot)

    rm -r ${tarball}
    rm ${tarball}.tar.gz
  fi
fi
