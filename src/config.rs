use std::path::{Component, Path, PathBuf};
use path_clean::PathClean;

#[track_caller]
fn throw_or<T,E: std::fmt::Debug>(e: Result<T,E>, code: i32, msg: Option<&str>) -> T {

    match e {
        Ok(r) => r,
        Err(e) => {
            if let Some(msg) = msg {
                eprintln!("{msg}");
            }
            eprintln!("{e:#?}");
            eprintln!("{}",std::panic::Location::caller());
            std::process::exit(code);
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedCfg {
    /// Files in here will never be modified. Files in dirs here will not be modified.
    never: Vec<PathBuf>,
    /// Locations in here will never be created (dirs will not be created regardless).
    no_create: Vec<PathBuf>
}

fn normalize_path<P: AsRef<Path>>(path: P) -> PathBuf {
    use path_clean::PathClean;
    let path = path.as_ref().to_path_buf();
    let path = PathBuf::from(&*shellexpand::full(path.to_str().unwrap()).unwrap());

    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap().join(path)
    }.clean();

    absolute_path
}

fn resolve_path<P: AsRef<Path>>(origin_path: P) -> std::io::Result<PathBuf> {
    let path = normalize_path(origin_path.as_ref());
    let mut resolved = PathBuf::new();

    for c in path.components() {
        match c {
            Component::RootDir => {resolved.push("/")}
            Component::Normal(p) => {
                let mut tmp = resolved.clone();
                tmp.push(p);
                if tmp.is_symlink() {
                    let sym = tmp.read_link()?;
                    if sym.is_absolute() {
                        resolved = sym;
                    } else {
                        resolved.push(sym);
                    }
                } else {
                    resolved.push(p)
                }
            }

            _ => panic!("Path not normalized")
        }
    }

    if resolved.is_symlink() {
        let sym = resolved.read_link()?;
        if sym.is_absolute() {
            resolved = sym;
        } else {
            resolved.pop();
            resolved.push(sym);
        }
    }

    let resolved = resolved.clean();

    #[cfg(debug)]
    eprintln!("Resolved {} into {}", origin_path.as_ref().display(), resolved.display());

    Ok(resolved)
}

impl ParsedCfg {
    pub fn new() -> Self {
        Self {
            never: Vec::new(),
            no_create: Vec::new(),
        }
    }

    pub fn load(&mut self, path: PathBuf) {
        let file = throw_or(std::fs::read_to_string(&path),0x30, Some(&format!("Unable to read {}: ", path.display())));
        let mut tgt = None;
        for i in file.split('\n') {
            match i {
                "[never-ever]" => tgt = Some(&mut self.never),
                "[no-create]" => tgt = Some(&mut self.no_create),
                f if tgt.is_some() => {
                    if let Ok(p) = resolve_path(PathBuf::from(f)) {
                        let t = tgt.as_mut().unwrap();
                        t.push(p);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn can_write(&self, path: &PathBuf) -> Result<bool,std::io::Error> {
        let cannon = path.canonicalize()?;
        for i in self.never.iter() {
            if i.starts_with(&cannon) {
                return Ok(false)
            }
        }

        Ok(true)
    }

    pub fn can_create(&self, path: &PathBuf) -> Result<bool, std::io::Error> {
        let cannon = normalize_path(&path);
        for i in self.no_create.iter() {
            if cannon.starts_with(i) {
                return Ok(false)
            }
        }

        Ok(true)
    }
}

