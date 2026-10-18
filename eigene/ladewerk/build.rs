// Release-Build für Windows: werk-prebuild.sh legt 7z.exe, 7z.dll und License.txt (7-Zip von
// Igor Pavlov, 7-zip.org) nach vendor/ und setzt WERK_7Z_DIR. Dann werden sie in die
// Programmdatei eingebettet; ohne die Variable nimmt Ladewerk ein installiertes 7-Zip,
// WinRAR oder tar.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(embedded_7z)");
    println!("cargo::rerun-if-env-changed=WERK_7Z_DIR");
    if let Ok(dir) = std::env::var("WERK_7Z_DIR") {
        let dir = std::path::PathBuf::from(dir);
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        let files = [("LW_7Z_EXE", "7z.exe"), ("LW_7Z_DLL", "7z.dll"), ("LW_7Z_LICENSE", "License.txt")];
        if files.iter().all(|(_, f)| dir.join(f).is_file()) {
            println!("cargo::rustc-cfg=embedded_7z");
            for (var, f) in files {
                let p = dir.join(f);
                println!("cargo::rerun-if-changed={}", p.display());
                println!("cargo::rustc-env={}={}", var, p.display());
            }
        } else {
            println!("cargo::warning=WERK_7Z_DIR ohne 7z.exe/7z.dll/License.txt: {}", dir.display());
        }
    }
}
