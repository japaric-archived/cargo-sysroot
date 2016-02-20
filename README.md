[![Travis](https://travis-ci.org/japaric/cargo-sysroot.svg?branch=master)](https://travis-ci.org/japaric/cargo-sysroot)
[![Appveyor](https://ci.appveyor.com/api/projects/status/rm0cymdvbu5a89ja/branch/master?svg=true)](https://ci.appveyor.com/project/japaric/cargo-sysroot)

# `cargo-sysroot`

> Builds a sysroot with cross compiled standard crates

## The problem

Let's say you are building a `no_std` crate for a custom target, e.g. some Cortex M microcontroller.
You make that crate depend on [`rust-libcore`] to have Cargo cross compile the `core` crate as
part of the build process. `cargo build --target=$triple` works fine, and you get your cross
compiled crate.

[`rust-libcore`]: https://crates.io/crates/rust-libcore

Now let's say you want to depend on another `no_std` crate, like [`spin`], so you add it to your
dependencies, and call `cargo build --target=$target`. But you get:

[`spin`]: https://crates.io/crates/spin/

``` rust
   Compiling spin v0.3.5
error: can't find crate for `core` [E0463]
error: aborting due to previous error
```

`rust-libcore` builds a `core` crate that can be used by your crate, but not by `spin`. When Cargo
builds `spin`, it looks for the `core` crate in your Rust installation but there's none in it.

You have two alternatives:

You make the `spin` crate depend on `rust-libcore` and things just work. But, IMO, this is a bad
approach because (1) it's "viral", you'll have to make all the crates you want to use depend on
`rust-libcore`, and (2) if your crate depends on any other standard crate then more crates like
`rust-libstd` would need to be created.

Or you could install the cross compiled `core` crate in your Rust installation path. But, you may
not be able to due to permissions, or may not want to pollute your Rust installation.

## The solution

Enter `--sysroot`, this undocumented `rustc` feature let's you override the library search path.
This means you can create a "sysroot", which is like a minimal Rust installation with only
libraries, populate it with cross compiled crates and instruct `rustc` to use that sysroot instead
of the default Rust installation.

And `crate sysroot` does this for you. It takes cares of creating a sysroot with cross compiled
standard crates:

**NOTE** `cargo-sysroot` only works with nightly newer than 2016-02-12.

``` rust
# install the cargo sysroot subcommand
$ cargo install --git https://github.com/japaric/cargo-sysroot

# create a sysroot for $target in the directory target/sysroot
$ cargo sysroot --target $target target/sysroot
INFO: fetching source tarball
INFO: unpacking source tarball
INFO: creating .commit-hash file
INFO: symlinking host crates
INFO: building the core crate
   Compiling core v0.0.0 (file://...)
INFO: copy the core crate to the sysroot

# check the sysroot
$ tree target/sysroot/debug
target/sysroot/debug
├── lib
│   └── rustlib
│       ├── $target
│       │   └── lib
│       │       └── libcore.rlib
│       └── $host -> $RUST_INSTALLATION/lib/rustlib/$host
└── src
    ├── libcore
    │   │── lib.rs
    │   └── (...)
    ├── libstd
    │   │── lib.rs
    │   └── (...)
    └── (...)
```

With the sysroot in place you can use (*) the `RUSTFLAGS` env variable to make Cargo use the
sysroot:

(*) Support for `RUSTFLAGS` has not yet landed. See [rust-lang/cargo#2241].

[rust-lang/cargo#2241]: https://github.com/rust-lang/cargo/pull/2241

```
$ RUSTFLAGS='--sysroot target/sysroot/debug' cargo build --target $triple
   Compiling spin v0.3.5
   Compiling $crate v0.1.0 (file://...)
```

## Future

Right now `cargo sysroot` only cross compiles the `core` crate, but in the future it will be able
to compile any standard crate. This would make it easier to target systems where the Rust team
doesn't [provide] cross compiled standard crates. Rust's [new Cargo-based build system] will make
this easier to implement.

[provide]: http://static.rust-lang.org/dist/
[new Cargo-based build system]: https://github.com/rust-lang/rust/pull/31123

Once the standard crates become build-able with Cargo, I expect that they'll start exposing Cargo
features to allow customization.  I'm envisioning having `cargo sysroot` look at a `sysroot.toml`
file  that specifies the Cargo features each standard crate will be compiled with. Something like
this:

``` toml
# Customize the std crate for the mips-musl target
[mips-unknown-linux-musl.std]
# disables both backtraces, and statically linked jemalloc
default-features = false
# enables dynamically linked jemalloc
features = ["dynamic-jemalloc"]
```

`cargo-sysroot` will also need to cross compile C dependencies like `compiler-rt` and `jemalloc`.
The `sysroot.toml` could also expose keys that specify which cross compiler tools to use:

``` toml
[mips-unknown-linux-musl.build]
ar = "mips-openwrt-linux-ar"
gcc = "mips-openwrt-linux-gcc"
ranlib = "mips-openwrt-linux-ranlib"
```

Because `cargo-sysroot` is not coupled to the `cargo build` process, it would be possible to build
the sysroot using the nightly channel, and then use the sysroot with the stable channel.
`cargo-sysroot` would need to grow an option to build an specific checkout of Rust source, right now
it always checkout the source at the commit-hash provided by `rustc -Vv`.

I'd like to better integrate `cargo-sysroot` with other tools. I know that `multirust` [plans] to
provide a command to fetch Rust source code, `cargo-sysroot` could use the source code fetched by
`multirust` instead of fetching the code itself. And I'd like `cargo-sysroot` to become more
"transparent", instead of having to set an environment variable, I'd like to integrate it with Cargo
via `.cargo/config` to make Cargo pass `--sysroot` to `rustc` automatically, and even pick a
different sysroot based on the profile (debug vs release).

[plans]: https://github.com/brson/multirust/issues/77

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the
work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
