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

enum Target<'a> {
    /// `path/to/some/target.json`
    Spec(&'a Path),
    /// `arm-unknown-linux-gnueabihf`
    Triple(&'a str),
}

struct Context<'a> {
    commit_hash: &'a str,
    host: &'a str,
    out_dir: &'a Path,
    release: bool,
    target: Target<'a>,
    verbose: bool,
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
                                          .arg(Arg::with_name("verbose")
                                                   .help("Verbose cargo builds")
                                                   .long("verbose"))
                                          .author("Jorge Aparicio <japaricious@gmail.com>"))
                          .version(env!("CARGO_PKG_VERSION"))
                          .get_matches();

    if let Some(matches) = matches.subcommand_matches("sysroot") {
        if let (Some(target), Some(out_dir)) = (matches.value_of("triple"),
                                                matches.value_of("out_dir")) {
            if host == target {
                panic!("`cargo sysroot` for host has not been implement yet");
            }

            let target = if target.ends_with("json") {
                Target::Spec(Path::new(target))
            } else {
                Target::Triple(target)
            };

            let ref ctx = Context {
                commit_hash: commit_hash,
                host: host,
                out_dir: Path::new(out_dir),
                release: matches.is_present("release"),
                target: target,
                verbose: matches.is_present("verbose"),
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

    let (ref triple, ref spec_file): (String, _) = match ctx.target {
        Target::Spec(path) => {
            let path = try!(fs::canonicalize(path));
            let triple = path.file_stem().unwrap().to_str().unwrap().into();

            (triple, path)
        }
        Target::Triple(triple) => (triple.into(), PathBuf::from(format!("{}.json", triple))),
    };

    let mut copied_spec_file = false;
    if spec_file.exists() {
        let dst = src_dir.join(format!("{}.json", triple));

        info!("copy target specification file");
        try!(fs::copy(spec_file, &dst));
        copied_spec_file = true;
    }

    info!("building the core crate");
    let mut cmd = Command::new("cargo");
    cmd.args(&["build", "--target", triple]);
    if ctx.release {
        cmd.arg("--release");
    }
    if ctx.verbose {
        cmd.arg("--verbose");
    }
    assert!(try!(cmd.current_dir(src_dir).env("CARGO_TARGET_DIR", temp_dir).status()).success());

    if copied_spec_file {
        info!("delete target specification file");
        try!(fs::remove_file(spec_file));
    }

    info!("copy the core crate to the sysroot");
    let ref libdir = ctx.out_dir.join(format!("lib/rustlib/{}/lib", triple));
    try!(fs::create_dir_all(libdir));

    let ref src = temp_dir.join(format!("{}/{}/libcore.rlib",
                                        triple,
                                        if ctx.release {
                                            "release"
                                        } else {
                                            "debug"
                                        }));
    let ref dst = libdir.join("libcore.rlib");
    try!(fs::copy(src, dst));
}
