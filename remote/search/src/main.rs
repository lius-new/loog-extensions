//! Plugin-private target scanner. All filesystem access descends through retained
//! directory descriptors; links and special files are never followed or read.
#[cfg(target_os = "linux")]
mod scan;

fn main() {
    #[cfg(target_os = "linux")]
    if let Err(error) = scan::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "linux"))]
    panic!("The bundled scanner targets Linux");
}
