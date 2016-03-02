extern crate tempdir;

use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::process::Command;
use std::{env, fs};

use tempdir::TempDir;

macro_rules! t {
    ($e:expr) => (match $e {
        Ok(e) => e,
        Err(e) => panic!("{} failed with {}", stringify!($e), e),
    })
}

fn cargo_sysroot() -> Command {
    let mut me = t!(env::current_exe());
    me.pop();
    me.push("cargo-sysroot");
    let mut cmd = Command::new(me);
    cmd.arg("sysroot");
    return cmd
}

fn exists_rlib(krate: &str, profile: &str, target: &str, sysroot: &Path) -> bool {
    for entry in t!(fs::read_dir(sysroot.join(format!("{}/lib/rustlib/{}/lib", profile, target)))) {
        let path = t!(entry).path();
        let filename = path.file_stem().unwrap().to_str().unwrap();
        let extension = path.extension().unwrap().to_str().unwrap();

        if filename.starts_with(&format!("lib{}", krate)) && extension == "rlib" && path.is_file() {
            return true;
        }
    }

    false
}

#[test]
fn supported_triple() {
    let triple = "arm-unknown-linux-gnueabihf";
    let td = t!(TempDir::new("cargo-sysroot"));

    run(cargo_sysroot().arg("--target")
                       .arg(triple)
                       .arg(td.path())
                       .arg("--verbose"));

    assert!(exists_rlib("core", "debug", triple, td.path()));

    run(cargo_sysroot().arg("--target")
                       .arg(triple)
                       .arg(td.path())
                       .arg("--verbose")
                       .arg("--release"));

    assert!(exists_rlib("core", "debug", triple, td.path()));
    assert!(exists_rlib("core", "release", triple, td.path()));
}

#[test]
fn custom_target() {
    let spec = r#"
        {
          "arch": "arm",
          "llvm-target": "thumbv7m-none-eabi",
          "os": "none",
          "target-endian": "little",
          "target-pointer-width": "32",
          "archive-format": "gnu"
        }
    "#;
    let td = t!(TempDir::new("cargo-sysroot"));
    t!(t!(File::create(td.path().join("custom.json"))).write_all(spec.as_bytes()));

    // test --target triple
    run(cargo_sysroot().arg("--target=custom")
                       .arg(td.path().join("target"))
                       .arg("--verbose")
                       .current_dir(td.path()));

    assert!(exists_rlib("core", "debug", "custom", &td.path().join("target")));

    // test /path/to/target.json
    run(cargo_sysroot().arg("--target")
                       .arg(td.path().join("custom.json"))
                       .arg(td.path().join("other"))
                       .arg("--verbose"));

    assert!(exists_rlib("core", "debug", "custom", &td.path().join("other")));

    // make sure the original spec is there but the copied version is gone
    assert!(td.path().join("custom.json").is_file());
    assert!(!td.path().join("other/src/libcore/custom.json").is_file());
}

fn run(cmd: &mut Command) {
    println!("running: {:?}", cmd);
    let output = t!(cmd.output());
    if !output.status.success() {
        println!("--- stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        println!("--- stderr:\n{}", String::from_utf8_lossy(&output.stderr));
        panic!("expected success, got: {}", output.status);
    }
}
