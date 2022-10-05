//! Small tool to move QEMU and all of it's dynamic libraries into a chroot
//! for x86-64 Linux targets
//!
//! This tool will use `ldd` to determine the runtime dependencies of QEMU and
//! copy QEMU and all of these dependencies into the specified chroot
//! environment. This assists in using dynamically built QEMU inside of a
//! different architecture's chroot.
//!
//! This is designed to conflict with existing binaries in the chroot minimally
//! by installing the dependencies into `/lib64/x86_64`, which is specific to
//! x86-64 programs.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Wrapper type around [`Error`]
type Result<T> = std::result::Result<T, Error>;

/// Error types
#[derive(Debug)]
enum Error {
    /// Invalid arguments specified to the program
    InvalidArgs,

    /// QEMU binary didn't seem valid
    InvalidQemuPath,

    /// Chroot directory didn't seem valid
    InvalidChrootPath,

    /// Executing `ldd` failed
    RunLddFailed(std::io::Error),

    /// `ldd` returned an error
    LddError(Option<i32>),

    /// `ldd`'s stdout returned invalid utf-8
    LddInvalidUtf8(std::str::Utf8Error),

    /// Our very low-quality parser for `ldd` output failed
    UnexpectedLddOutput,

    /// Failed to canonicalize a library path
    LibCanonicalize(std::io::Error),

    /// Failed to create directory in chroot folder where library needed to be
    /// placed
    CreateOutputDirectory(PathBuf, std::io::Error),

    /// Failed to copy dependency into chroot
    CopyFile(PathBuf, PathBuf, std::io::Error),
}

/// Entry point
fn main() -> Result<()> {
    // Get the arguments
    let args: Vec<String> = std::env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        println!("usage: qemu_chrooter <path to QEMU binary> \
            <path to chroot>");
        return Err(Error::InvalidArgs);
    }

    // Get the QEMU path
    let qemu = Path::new(&args[1]);
    if !qemu.is_file() {
        println!("QEMU binary doesn't seem to be a valid file!");
        return Err(Error::InvalidQemuPath);
    }

    // Get the chroot path
    let chroot = Path::new(&args[2]);
    if !chroot.is_dir() {
        println!("chroot doesn't seem to be a valid directory!");
        return Err(Error::InvalidChrootPath);
    }

    // Determine dependencies for QEMU using `ldd`
    let ldd_res = Command::new("ldd").arg(&args[1]).output()
        .map_err(Error::RunLddFailed)?;
    if !ldd_res.status.success() {
        return Err(Error::LddError(ldd_res.status.code()));
    }
    let ldd_stdout = std::str::from_utf8(&ldd_res.stdout)
        .map_err(Error::LddInvalidUtf8)?;

    // Parse out dependencies
    let mut libs = Vec::new();
    let mut loader = None;
    for line in ldd_stdout.lines() {
        if line.contains(" => ") {
            // A normal library line, ala
            // `        libm.so.6 => /lib64/libm.so.6 (0x00007f257b923000)`
            let lib = line.splitn(2, " => ").nth(1)
                .and_then(|x| x.rsplitn(2, " (0x").nth(1))
                .ok_or(Error::UnexpectedLddOutput)?;
            libs.push(lib);
        } else if line.contains("linux-vdso.so") {
            // A Linux VDSO line. Doesn't actually get loaded, ignore it
            continue;
        } else if line.contains("ld-linux") {
            // Dynamic linker. Required to run the binary, doesn't contain
            // the ` => ` as a normal line, so we have to parse it differently
            if loader.is_some() {
                println!("Whoa, two loaders in the LDD output?");
                return Err(Error::UnexpectedLddOutput);
            }

            // Save off the loader we found
            loader = Some(line.rsplitn(2, " (0x").nth(1).map(|x| x.trim())
                .ok_or(Error::UnexpectedLddOutput)?);
        }
    }

    // Make sure we had a loader
    if loader.is_none() {
        println!("No dynamic loader found for binary, is the binary \
            statically linked?");
        return Err(Error::UnexpectedLddOutput);
    }

    // Make sure everything we found is a file, and perform the copy
    for (custom_path, lib) in
            libs.iter().map(|x| (Some("lib64/x86_64"), x))
            .chain(Some((Some("usr/bin"), &args[1].as_str())))
            .chain(loader.iter().map(|x| (None, x))) {
        let lib_path = Path::new(lib).canonicalize()
            .map_err(Error::LibCanonicalize)?;
        if !lib_path.is_file() {
            println!("Dynamic dependency is not a valid file: {lib}");
            return Err(Error::UnexpectedLddOutput);
        }

        // Get the folder where the library is contained
        let target_dir = if let Some(custom_path) = custom_path {
            Path::new(custom_path)
        } else {
            // Loader must be put in the _exact_ path specified in the chroot
            // perspective, so we don't put it in `/lib64/x86_64` as we put the
            // others
            let parent = lib_path.parent().unwrap();
            parent.strip_prefix("/").unwrap()
        };

        // Determine directory where we will be placing the file
        let dest = chroot.join(target_dir);

        // Make sure the target path exists
        std::fs::create_dir_all(&dest)
            .map_err(|x| Error::CreateOutputDirectory(dest.clone(), x))?;

        // Copy file
        let dest_file = dest.join(Path::new(lib).file_name().unwrap());
        println!("Copying {:?} -> {:?}", lib_path, dest_file);
        std::fs::copy(&lib_path, &dest_file)
            .map_err(|x| {
                Error::CopyFile(lib_path.clone(), dest_file.clone(), x)
            })?;
    }

    // Copy QEMU itself

    Ok(())
}

