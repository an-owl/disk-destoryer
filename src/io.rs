use std::os::fd::FromRawFd;

/// Contains info on the target which can be used to open a file.
#[derive(Clone, Debug)]
pub enum Target {
    Path(std::path::PathBuf),
    StdIn,
    StdOut,
}

const FD_STDIN: std::os::fd::RawFd = 0;
const FD_STDOUT: std::os::fd::RawFd = 1;

/// Exists the process printing some info on the problem
fn handle_fs_err(path: &std::path::Path, err: std::io::Error) -> ! {
    eprint!("Failed to open {}",path.display());
    let mut unknown = true;
    if let Some(os) = err.raw_os_error() {
        eprint!(": OS Error {}", os);
        unknown = false;
    }
    let msg = err.to_string();
    if msg.len() > 0 {
        if unknown {
            eprint!(": {msg}");
        } else {
            eprint!(", {msg}");
        }
    }
    eprint!("\n");
    std::process::exit(0x11);
}
#[derive(Eq, PartialEq)]
pub enum IoMode {
    Read,
    Write,
}

impl Target {
    pub fn open(&self, mode: IoMode) -> std::fs::File {
        match self {
            Target::Path(p) => {
                if mode == IoMode::Write {
                    let mut o = std::fs::OpenOptions::new();
                    o.write(true).create(true);
                    o.open(p).unwrap_or_else(|e| super::handle_err(e, &format!("in file {self:?}"),0x10))
                } else {
                    std::fs::File::open(p).unwrap_or_else(|e| handle_fs_err(&p,e) )
                }
            }
            Target::StdIn => unsafe { std::fs::File::from_raw_fd(FD_STDIN) },
            Target::StdOut => unsafe { std::fs::File::from_raw_fd(FD_STDOUT) },
        }
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Path(p) => write!(f, "{}", p.display()),
            Target::StdIn => write!(f, "stdin"),
            Target::StdOut => write!(f, "stdout"),
        }
    }
}