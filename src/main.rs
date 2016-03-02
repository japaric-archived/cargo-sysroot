#![deny(warnings)]

extern crate chrono;
extern crate clap;
extern crate curl;
extern crate fern;
extern crate flate2;
extern crate rustc_version;
extern crate tar;
extern crate tempdir;
extern crate toml;

#[macro_use]
extern crate log;

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::NaiveDate;
use clap::{App, AppSettings, Arg, SubCommand};
use curl::http;
use rustc_version::Channel;

// TODO proper error reporting
macro_rules! try {
    ($e:expr) => {
        $e.unwrap_or_else(|e| panic!("{} with {}", stringify!($e), e))
    }
}

enum Target<'a> {
    /// `path/to/some/target.json`
    Spec(&'a Path),
    /// `arm-unknown-linux-gnueabihf`
    Triple(&'a str),
}

struct Context<'a> {
    commit_date: NaiveDate,
    commit_hash: &'a str,
    host: &'a str,
    out_dir: &'a Path,
    profile: &'static str,
    target: Target<'a>,
    verbose: bool,
}

struct Config {
    table: Option<toml::Value>,
}

impl Config {
    /// Parses `sysroot.toml`
    fn parse() -> Config {
        let path = Path::new("sysroot.toml");

        let table = if path.exists() {
            let ref mut toml = String::new();

            try!(try!(File::open(path)).read_to_string(toml));

            toml::Parser::new(toml).parse().map(toml::Value::Table)
        } else {
            info!("no sysroot.toml found, using default configuration");

            None
        };

        Config { table: table }
    }

    /// Crates to build for this target, returns `["core"]` if not specified
    fn crates(&self, target: &str) -> Vec<String> {
        let ref key = format!("target.{}.crates", target);

        let mut crates = self.table
                             .as_ref()
                             .and_then(|t| t.lookup(key))
                             .and_then(|v| v.as_slice())
                             .and_then(|vs| {
                                 vs.iter().map(|v| v.as_str().map(|s| s.to_owned())).collect()
                             })
                             .unwrap_or_else(|| vec!["core".to_owned()]);

        crates.push("core".to_owned());
        crates.sort();
        crates.dedup();
        crates
    }
}

fn main() {
    let rustc_version::VersionMeta { ref host, ref commit_date, ref commit_hash, ref channel, .. } =
        rustc_version::version_meta();
    let commit_hash = commit_hash.as_ref().unwrap();

    match *channel {
        Channel::Nightly => {}
        _ => panic!("only the nightly channel is supported at this time (see issue #5)"),
    }

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
                commit_date: NaiveDate::parse_from_str(commit_date.as_ref().unwrap(), "%Y-%m-%d")
                                 .unwrap(),
                commit_hash: commit_hash,
                host: host,
                out_dir: Path::new(out_dir),
                profile: if matches.is_present("release") {
                    "release"
                } else {
                    "debug"
                },
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

// TODO Ultimately, we want to use `multirust fetch-source` for 100% correctness
fn fetch_source(ctx: &Context) {
    // XXX There doesn't seem to be way to get the _nightly_ date from the output of `rustc -Vv`
    // So we _assume_ the nightly day is the day after the commit-date found in `rustc -Vv` which
    // seems to be the common case, but it could be wrong and we'll end up building unusable crates
    let date = ctx.commit_date.succ();
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
    let handle = http::Handle::new();
    let url = format!("http://static.rust-lang.org/dist/{}/rustc-nightly-src.tar.gz",
                      date.format("%Y-%m-%d"));
    let resp = try!(handle.timeout(300_000).get(&url[..]).follow_redirects(true).exec());

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
                continue;
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
    let ref rustlib_dir = ctx.out_dir.join(format!("{}/lib/rustlib", ctx.profile));
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
            try!(fs::hard_link(&src, &dst).or_else(|_| fs::copy(&src, &dst).map(|_| ())));
        }
    }
}

fn build_target_crates(ctx: &Context) {
    const LIB_RS: &'static [u8] = b"#![no_std]";

    let ref config = Config::parse();

    let temp_dir = try!(tempdir::TempDir::new("sysroot"));
    let temp_dir = temp_dir.path();

    // Create Cargo project
    let ref mut cargo = Command::new("cargo");
    cargo.current_dir(temp_dir);
    cargo.args(&["new", "--vcs", "none"]);
    if ctx.verbose {
        cargo.arg("--verbose");
    }
    assert!(try!(cargo.arg("sysroot").status()).success());

    let ref cargo_dir = temp_dir.join("sysroot");
    let ref src_dir = env::current_dir().unwrap().join(ctx.out_dir).join("src");

    let (ref triple, ref spec_file): (String, _) = match ctx.target {
        Target::Spec(path) => {
            let path = try!(fs::canonicalize(path));
            let triple = path.file_stem().unwrap().to_str().unwrap().into();

            (triple, path)
        }
        Target::Triple(triple) => (triple.into(), PathBuf::from(format!("{}.json", triple))),
    };

    // Add crates to build as dependencies
    let ref mut toml = try!(OpenOptions::new()
                                .write(true)
                                .append(true)
                                .open(cargo_dir.join("Cargo.toml")));
    let ref crates = config.crates(triple);
    info!("will build the following crates: {:?}", crates);
    for ref krate in crates {
        try!(writeln!(toml,
                      "{} = {{ path = '{}' }}",
                      krate,
                      src_dir.join(format!("lib{}", krate)).display()))
    }

    {
        let ref mut toml = String::new();
        try!(try!(File::open(cargo_dir.join("Cargo.toml"))).read_to_string(toml));
        debug!("sysroot's Cargo.toml: {}", toml);
    }

    // Rewrite lib.rs to only depend on libcore
    try!(try!(OpenOptions::new().write(true).truncate(true).open(cargo_dir.join("src/lib.rs")))
             .write_all(LIB_RS));

    if spec_file.exists() {
        info!("copy target specification file");
        try!(fs::copy(spec_file, cargo_dir.join(format!("{}.json", triple))));
    }

    info!("building the target crates");
    let ref mut cargo = Command::new("cargo");
    cargo.current_dir(cargo_dir);
    cargo.args(&["build", "--target", triple]);
    if ctx.profile == "release" {
        cargo.arg("--release");
    }
    if ctx.verbose {
        cargo.arg("--verbose");
    }
    assert!(try!(cargo.status()).success());

    info!("copy the target crates to the sysroot");
    let ref libdir = ctx.out_dir.join(format!("{}/lib/rustlib/{}/lib", ctx.profile, triple));
    let ref deps_dir = cargo_dir.join(format!("target/{}/{}/deps", triple, ctx.profile));

    try!(fs::create_dir_all(libdir));
    for entry in try!(fs::read_dir(deps_dir)) {
        let entry = try!(entry);

        try!(fs::copy(entry.path(), libdir.join(entry.file_name())));
    }
}
