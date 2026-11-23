use std::{env, path::Path, process::Command};

// git archive expands this field even when the packaged sources have no .git directory.
const ARCHIVE_REVISION: &str = "$Format:%H$";

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn revision(root: &Path) -> String {
    if ARCHIVE_REVISION.bytes().all(|b| b.is_ascii_hexdigit()) {
        return ARCHIVE_REVISION.into();
    }
    if root.join(".git").exists() {
        let branch = git(root, &["symbolic-ref", "-q", "HEAD"]);
        for entry in ["HEAD", "packed-refs"].into_iter().chain(branch.as_deref()) {
            if let Some(path) = git(
                root,
                &["rev-parse", "--path-format=absolute", "--git-path", entry],
            ) {
                if Path::new(&path).is_file() {
                    println!("cargo:rerun-if-changed={path}");
                }
            }
        }
        if let Some(head) = git(root, &["rev-parse", "--verify", "HEAD"]) {
            return head;
        }
    }
    "unknown".into()
}

fn windows_icon() {
    println!("cargo:rerun-if-changed=assets/phyro.ico");
    println!("cargo:rerun-if-changed=phyro.rc");
    println!("cargo:rerun-if-env-changed=RC");
    let output = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let compiler = env::var_os("RC");
    let (mut command, resource) = if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let sdk_rc = || {
            let arch = env::var("CARGO_CFG_TARGET_ARCH").ok()?;
            find_msvc_tools::find_windows_sdk(&arch)?
                .path()
                .map(|dir| dir.join("rc.exe"))
                .find(|exe| exe.is_file())
                .map(|exe| exe.into_os_string())
        };
        let mut command = Command::new(compiler.or_else(sdk_rc).unwrap_or_else(|| "rc.exe".into()));
        let resource = output.join("phyro.res");
        command
            .arg("/nologo")
            .arg("/fo")
            .arg(&resource)
            .arg("phyro.rc");
        (command, resource)
    } else {
        let resource = output.join("phyro-icon.o");
        let mut command = Command::new(compiler.unwrap_or_else(|| "windres".into()));
        command
            .args(["-i", "phyro.rc", "-O", "coff", "-o"])
            .arg(&resource);
        (command, resource)
    };
    assert!(
        command
            .status()
            .expect("Run the Windows resource compiler")
            .success(),
        "Failed to compile the Phyro application icon"
    );
    println!("cargo:rustc-link-arg-bin=phyro={}", resource.display());
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_icon();
    }
    let manifest = env::var_os("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&manifest).join("../..");
    println!("cargo:rustc-env=PHYRO_BUILD={}", revision(&root));
    println!(
        "cargo:rustc-env=PHYRO_TARGET={}",
        env::var("TARGET").unwrap()
    );
}
