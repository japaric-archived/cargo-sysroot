#![deny(warnings)]

extern crate clap;
extern crate curl;
extern crate fern;
extern crate flate2;
extern crate rustc_version;
extern crate tar;
extern crate tempdir;

#[macro_use]
extern crate log;

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{App, AppSettings, Arg, SubCommand};
use curl::http;

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

    info!("fetching source tarball");
    let mut handle = http::Handle::new();
    let url = format!("https://github.com/rust-lang/rust/tarball/{}", hash);
    let resp = try!(handle.get(&url[..]).follow_redirects(true).exec());

    assert_eq!(resp.get_code(), 200);

    info!("unpacking source tarball");
    let decoder = try!(flate2::read::GzDecoder::new(resp.get_body()));
    let mut archive = tar::Archive::new(decoder);
    for entry in try!(archive.entries()) {
        let mut entry = try!(entry);
        let path = {
            let path = try!(entry.path());
            let mut components = path.components();
            components.next(); // skip rust-lang-rust-<...>
            let next = components.next().and_then(|s| s.as_os_str().to_str());
            if next != Some("src") {
                continue
            }
            components.as_path().to_path_buf()
        };
        try!(entry.unpack(&src_dir.join(path)));
    }

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
        try!(fs::remove_dir_all(dst));
    }

    link_dirs(&src, &dst);
}

fn link_dirs(src: &Path, dst: &Path) {
    try!(fs::create_dir(&dst));

    for entry in try!(src.read_dir()) {
        let entry = try!(entry);
        let src = entry.path();
        let dst = dst.join(entry.file_name());
        if try!(entry.file_type()).is_dir() {
            link_dirs(&src, &dst);
        } else {
            try!(fs::hard_link(&src, &dst).or_else(|_| {
                fs::copy(&src, &dst).map(|_| ())
            }));
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

    let mut copied_spec_file = None;
    if spec_file.exists() {
        let dst = src_dir.join(format!("{}.json", triple));

        info!("copy target specification file");
        try!(fs::copy(spec_file, &dst));
        copied_spec_file = Some(dst);
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

    if let Some(file) = copied_spec_file {
        info!("delete target specification file");
        try!(fs::remove_file(file));
    }

    info!("copy the core crate to the sysroot");
    let profile = if ctx.release { "release" } else { "debug" };
    let ref libdir = ctx.out_dir.join(format!("{}/lib/rustlib/{}/lib", profile, triple));
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
