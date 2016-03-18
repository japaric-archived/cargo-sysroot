# `before_deploy` phase: here we package the build artifacts

set -ex

cargo build --target $TARGET --release

mkdir staging

cp target/$TARGET/release/cargo-sysroot staging

cd staging

tar czf ../${PROJECT_NAME}-${TRAVIS_TAG}-${TARGET}.tar.gz *
