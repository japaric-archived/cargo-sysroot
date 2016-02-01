#![deny(warnings)]

extern crate clap;
extern crate fern;
extern crate rustc_version;
extern crate tempdir;

#[macro_use]
extern crate log;

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::{App, AppSettings, Arg, SubCommand};

// TODO proper error reporting
macro_rules! try {
    ($e:expr) => {
        $e.unwrap_or_else(|_| panic!(stringify!($e)))
    }
}

struct Context<'a> {
    commit_hash: &'a str,
    host: &'a str,
    out_dir: &'a Path,
    release: bool,
    target: &'a str,
}

fn main() {
    let rustc_version::VersionMeta { ref host, ref commit_hash, .. } =
        rustc_version::version_meta();
    let commit_hash = commit_hash.as_ref().unwrap();

    let ref matches = App::new("cargo-sysroot")
                          .bin_name("cargo")
                          .settings(&[AppSettings::SubcommandRequired])
                          .subcommand(SubCommand::with_name("sysroot")
                                          .about("Builds a sysroot with cross compiled standard \
                                                  crates")
                                          .arg(Arg::with_name("triple")
                                                   .help("Target triple to compile for")
                                                   .long("target")
                                                   .required(true)
                                                   .takes_value(true))
                                          .arg(Arg::with_name("release")
                                                   .help("Build artifacts in release mode, \
                                                          with optimizations")
                                                   .long("release"))
                                          .arg(Arg::with_name("out_dir")
                                                   .help("Output directory")
                                                   .required(true))
                                          .author("Jorge Aparicio <japaricious@gmail.com>"))
                          .version(env!("CARGO_PKG_VERSION"))
                          .get_matches();

    if let Some(matches) = matches.subcommand_matches("sysroot") {
        if let (Some(target), Some(out_dir)) = (matches.value_of("triple"),
                                                matches.value_of("out_dir")) {
            if host == target {
                panic!("`cargo sysroot` for host has not been implement yet");
            }

            let ref ctx = Context {
                commit_hash: commit_hash,
                host: host,
                out_dir: Path::new(out_dir),
                release: matches.is_present("release"),
                target: target,
            };

            init_logger();
            fetch_source(ctx);
            symlink_host_crates(ctx);
            build_target_crates(ctx);
        }
    }
}

fn init_logger() {
    let config = fern::DispatchConfig {
        format: Box::new(|msg, level, _| format!("{}: {}", level, msg)),
        output: vec![fern::OutputConfig::stderr()],
        level: log::LogLevelFilter::Trace,
    };

    try!(fern::init_global_logger(config, log::LogLevelFilter::Trace));
}

fn fetch_source(ctx: &Context) {
    let hash = ctx.commit_hash;
    let ref src_dir = ctx.out_dir.join("src");
    let ref hash_file = src_dir.join(".commit-hash");

    if hash_file.exists() {
        let ref mut old_hash = String::with_capacity(hash.len());
        try!(try!(File::open(hash_file)).read_to_string(old_hash));

        if old_hash == hash {
            info!("source up to date");
            return;
        }
    }

    if src_dir.exists() {
        info!("purging the src directory");
        try!(fs::remove_dir_all(src_dir));
    }

    try!(fs::create_dir_all(src_dir));

    // FIXME don't rely on `curl` and `tar`, instead implement this functionality in Rust.
    // Check https://github.com/Diggsey/multirust-rs for a reference implementation of such
    // functionality
    info!("fetching source tarball");
    let ref curl = try!(Command::new("curl")
                            .arg("-L")
                            .arg(format!("https://github.com/rust-lang/rust/tarball/{}", hash))
                            .output());

    assert!(curl.status.success());

    info!("unpacking source tarball");
    let short_hash = &hash[..7];
    let ref mut tar = try!(Command::new("tar")
                               .args(&["-xz", "--strip-components", "2", "-C"])
                               .arg(src_dir)
                               .arg(format!("rust-lang-rust-{}/src", short_hash))
                               .stdin(Stdio::piped())
                               .spawn());
    try!(tar.stdin.as_mut().unwrap().write_all(&curl.stdout));

    info!("creating .commit-hash file");
    try!(try!(File::create(hash_file)).write_all(hash.as_bytes()));
}

fn symlink_host_crates(ctx: &Context) {
    info!("symlinking host crates");

    let sys_root = try!(String::from_utf8(try!(Command::new("rustc")
                                                   .args(&["--print", "sysroot"])
                                                   .output())
                                              .stdout));
    let ref src = Path::new(sys_root.trim_right()).join(format!("lib/rustlib/{}", ctx.host));
    let ref rustlib_dir = ctx.out_dir.join("lib/rustlib");
    let ref dst = rustlib_dir.join(ctx.host);

    try!(fs::create_dir_all(rustlib_dir));

    if dst.exists() {
        try!(fs::remove_file(dst));
    }

    match () {
        #[cfg(unix)]
        () => {
            use std::os::unix::fs;

            try!(fs::symlink(src, dst))
        }
        #[cfg(windows)]
        () => {
            use std::os::windaws::fs;

            try!(fs::symlink_dir(src, dst))
        }
    }
}

// XXX we only build the core crate for now
// TODO build the rest of crates
fn build_target_crates(ctx: &Context) {
    const CARGO_TOML: &'static str = r#"[package]
authors = ["The Rust Project Developers"]
name = "core"
version = "0.0.0"

[lib]
name = "core"
path = "lib.rs""#;

    let ref src_dir = ctx.out_dir.join("src/libcore");

    try!(try!(File::create(src_dir.join("Cargo.toml"))).write_all(CARGO_TOML.as_bytes()));

    let ref temp_dir = try!(tempdir::TempDir::new("core"));
    let temp_dir = temp_dir.path();

    let ref target_json = PathBuf::from(format!("{}.json", ctx.target));
    let mut spec_file = None;

    if target_json.exists() {
        let dst = src_dir.join(target_json);

        info!("copy target specification file");
        try!(fs::copy(target_json, &dst));
        spec_file = Some(dst);
    }

    info!("building the core crate");
    assert!(try!(Command::new("cargo")
                     .args(&["build", "--target"])
                     .arg(ctx.target)
                     .arg(if ctx.release {
                         "--release"
                     } else {
                         // XXX dummy value
                         "--lib"
                     })
                     .current_dir(src_dir)
                     .env("CARGO_TARGET_DIR", temp_dir)
                     .status())
                .success());

    if let Some(spec_file) = spec_file {
        info!("delete target specification file");
        try!(fs::remove_file(spec_file));
    }

    info!("copy the core crate to the sysroot");
    let ref libdir = ctx.out_dir.join(format!("lib/rustlib/{}/lib", ctx.target));
    try!(fs::create_dir_all(libdir));

    let ref src = temp_dir.join(format!("{}/{}/libcore.rlib",
                                        ctx.target,
                                        if ctx.release {
                                            "release"
                                        } else {
                                            "debug"
                                        }));
    let ref dst = libdir.join("libcore.rlib");
    try!(fs::copy(src, dst));
}
