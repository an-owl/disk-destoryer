use std::env;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};

const DEFAULT_CONFIG: &str = "/etc/disk-destroyer.conf";
static STATE: GlobalState = GlobalState::new();

// rc's
// 0x0?: See Options::new
// 0x1?: FS problem
// 0x2?: IO problem see read_write
// 0x3?: Config Error
// 0x130: sigint

mod io;
mod read_write;
mod config;

fn main() {
    let o = Options::new();
    let (tx,rx) = std::sync::mpsc::channel();

    unsafe { signal_hook::low_level::register(signal_hook::consts::SIGINT, sigint).expect("Error installing interrupt handler") };
    unsafe { signal_hook::low_level::register(signal_hook::consts::SIGUSR1, sigusr1).expect("Error installing interrupt handler") };

    STATE.cfg_time();

    let write_thread = {
        let options_send = o.clone();
        std::thread::spawn(move || read_write::dd_write(options_send,rx))
    };

    read_write::dd_read(o.clone(),tx);
    drop(write_thread.join());

    if o.status != Status::NoXFer {
        eprintln!("{}", STATE);
    }
}

fn sigint() {
    eprintln!("{}",STATE);
    std::process::exit(130);
}

fn sigusr1() {
    eprintln!("{}", STATE);
}

#[derive(Debug, Clone)]
pub struct Options {
    o_f: io::Target,
    i_f: io::Target,
    i_bs: usize,
    o_bs: usize,
    count: Option<usize>,
    o_skip: Option<usize>,
    i_skip: Option<usize>,
    status: Status,
    cfg: config::ParsedCfg
}

struct GlobalState {
    queued: AtomicUsize,

    read_blk: AtomicUsize,
    read_extra: AtomicBool,

    write_blk: AtomicUsize,
    write_extra: AtomicBool,
    // this should ever be poisoned. it is only locked by the main thread and during an interrupt.
    started: std::sync::RwLock<Option<std::time::SystemTime>>,
    bytes_written: AtomicUsize,
}

impl GlobalState {
    const fn new() -> Self {
        Self{
            queued: AtomicUsize::new(0),
            read_blk: AtomicUsize::new(0),
            read_extra: AtomicBool::new(false),
            write_blk: AtomicUsize::new(0),
            write_extra: AtomicBool::new(false),
            started: std::sync::RwLock::new(None),
            bytes_written: AtomicUsize::new(0),
        }
    }

    fn cfg_time(&self) {
        *self.started.write().unwrap() = Some(std::time::SystemTime::now())
    }
}

impl Display for GlobalState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}+{} records in", self.read_blk.load(std::sync::atomic::Ordering::Relaxed), self.read_extra.load(std::sync::atomic::Ordering::Relaxed) as u8)?;
        writeln!(f, "{}+{} records out" , self.write_blk.load(std::sync::atomic::Ordering::Relaxed), self.write_extra.load(std::sync::atomic::Ordering::Relaxed) as u8)?;
        let bytes = self.bytes_written.load(std::sync::atomic::Ordering::Relaxed);

        if let Some((pow, unit)) = match bytes as f64 {
            f if f/1024f64.powi(4) > 1f64 => Some((4,'T')),
            f if f/1024f64.powi(3) > 1f64 => Some((3,'G')),
            f if f/1024f64.powi(2) > 1f64 => Some((2,'M')),
            f if f/1024f64 > 1f64 => Some((1,'K')),
            _ => None,
        } {
            write!(f,"{bytes} bytes ({pow2:.1}{unit}iB {pow10:.1}{unit}B)", pow2 = (bytes as f64)/1024f64.powi(pow), pow10 = (bytes as f64)/1000f64.powi(pow))?;
        } else {
            write!(f, "{bytes} bytes copied")?;
        }



        if let Ok(d) = self.started.read().unwrap().unwrap().elapsed() {
            let mins = d.as_secs_f64() / 60f64;
            let hrs = d.as_secs_f64() / 60f64.powi(2);
            if hrs > 1.0 {
                write!(f, ", {hrs:.3} h")?;
            } else if mins > 1.0 {
                write!(f, ", {mins:.3} m")?;
            } else {
                write!(f, ", {:.3} s", d.as_secs_f64())?;
            }

            let bps = self.bytes_written.load(std::sync::atomic::Ordering::Relaxed) as f64 / d.as_secs_f64();
            let (bps, unit) = match bps {
                b if b > 1024f64.powi(4) => ( b/1024f64.powi(4), "TiB/s"),
                b if b > 1024f64.powi(3) => ( b/1024f64.powi(3), "GiB/s"),
                b if b > 1024f64.powi(2) => ( b/1024f64.powi(2), "MiB/s"),
                b => (b/1024f64, "KiB/s"),
            };

            write!(f, ", {bps:.2}{unit}")?;
        }
        writeln!(f,"")
    }
}

impl Options {
    const BRIEF: &'static str = "";
    fn new() -> Self {
        use getopts::{HasArg, Occur};
        let mut opts = getopts::Options::new();
        // gnu `dd` options
        opts.opt("", "if","read from FILE instead of stdin","FILE",HasArg::Yes,Occur::Optional);
        opts.opt("", "of","write to FILE instead of stdout", "FILE", HasArg::Yes,Occur::Optional);
        opts.opt("", "count","copy only N input blocks", "N", HasArg::Yes, Occur::Optional);
        opts.opt("", "bs","read and write up to BYTES bytes at a time (default: 512); overrides ibs and obs", "BYTES", HasArg::Yes, Occur::Optional);
        opts.opt("", "ibs","read up to BYTES bytes at a time (default: 512)","BYTES",HasArg::Yes,Occur::Optional);
        opts.opt("", "obs", "write BYTES bytes at a time (default: 512)", "BYTES", HasArg::Yes, Occur::Optional);
        opts.opt("", "seek","skip N obs-sized output blocks. Note: oseek will not work", "N",HasArg::Yes, Occur::Optional);
        opts.opt("", "skip", "skip N ibs-sized input blocks. Nose iseek does not work", "N", HasArg::Yes, Occur::Optional);
        opts.opt("", "status", "The LEVEL of information to print to stderr; 'none' suppresses everything but error messages, 'noxfer' suppresses the final transfer statistics, 'progress' shows periodic transfer statistics","LEVEL", HasArg::Yes, Occur::Optional);
        opts.opt("","no-cfg", "Disables parsing the default config", "", HasArg::No, Occur::Optional);

        // disk destroyer options
        opts.opt("","cfg", "points to the config file to b used","PATH", HasArg::Yes,Occur::Multi);
        opts.opt("","help", "Prints a useful help message","",HasArg::No,Occur::Optional);

        let matches = match opts.parse(env::args()) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to parse cmdline: {e:?}");
                std::process::exit(1);
            },
        };

        if matches.opt_present("help") {
            opts.usage(Self::BRIEF);
            std::process::exit(0);
        }

        let mut o_f = None;
        let mut i_f = None;
        let mut count = None;
        let mut o_skip = None;
        let mut i_skip = None;
        let mut status = None;
        let mut i_bs = None;
        let mut o_bs = None;
        let mut bs_spec = 0; // 0 for not configured yet. 1 for legacy opt. 2 for long opt.

        // todo prepend '--' and re-parse these to remove this section
        for arg in matches.free.iter().skip(1) {
            if arg.starts_with("if") {
                if i_f.is_none() {
                    i_f = Some(arg.split("=").nth(1).unwrap_or_else(|| {
                        eprintln!("Expected if=[INT], found: {arg}");
                        opts.usage(Self::BRIEF);
                        std::process::exit(2);
                    }).to_string())
                } else {
                    eprintln!("Multiple instances of \"if\" expected one");
                    std::process::exit(3);
                }
            } else if arg.starts_with("of") {
                if o_f.is_none() {
                    o_f = Some(arg.split("=").nth(1).unwrap_or_else(|| {
                        eprintln!("Expected of=[INT], found: {arg}");
                        opts.usage(Self::BRIEF);
                        std::process::exit(2);
                    }).to_string())
                } else {
                    eprintln!("Multiple instances of \"of\" expected one");
                    std::process::exit(3);
                }
            } else if arg.starts_with("count") {
                if count.is_none() {
                    count = Some(arg.split("=").nth(1).unwrap_or_else(|| {
                        eprintln!("Expected count=[INT], found: {arg}");
                        opts.usage(Self::BRIEF);
                        std::process::exit(2);
                    }).to_string())
                } else {
                    eprintln!("Multiple instances of \"count\" expected one");
                    std::process::exit(3);
                }
            } else if arg.starts_with("bs") {
                if bs_spec < 1 {
                    bs_spec = 1;
                    i_bs = Some(arg.split("=").nth(1).unwrap_or_else(|| {
                        eprintln!("Expected bs=[INT], found: {arg}");
                        opts.usage(Self::BRIEF);
                        std::process::exit(2);
                    }).to_string());
                    o_bs = i_bs.clone()
                } else {
                    eprintln!("Multiple instances of \"bs\" expected one");
                    std::process::exit(3);
                }
            } else if arg.starts_with("ibs") {
                if bs_spec < 1 {
                    if i_bs.is_none() {
                        i_bs = Some(arg.split("=").nth(1).unwrap_or_else(|| {
                            eprintln!("Expected ibs=[INT], found: {arg}");
                            opts.usage(Self::BRIEF);
                            std::process::exit(2);
                        }).to_string())
                    } else {
                        // dont exit if "bs" specified
                        if bs_spec == 0 {
                            eprintln!("Multiple instances of \"ibs\" expected one");
                            std::process::exit(3);
                        }
                    } // else skip
                }
            } else if arg.starts_with("obs") {
                if bs_spec < 1 {
                    if o_bs.is_none() {
                        o_bs = Some(arg.split("=").nth(1).unwrap_or_else(|| {
                            eprintln!("Expected obs=[INT], found: {arg}");
                            opts.usage(Self::BRIEF);
                            std::process::exit(2);
                        }).to_string())
                    } else {
                        if bs_spec == 0 {
                            eprintln!("Multiple instances of \"obs\" expected one");
                            std::process::exit(3);
                        }
                    }
                }
            } else if arg.starts_with("seek") {
                if i_skip.is_none() {
                    i_skip = Some(arg.split("=").nth(1).unwrap_or_else(|| {
                        eprintln!("Expected seek=[INT], found: {arg}");
                        opts.usage(Self::BRIEF);
                        std::process::exit(2);
                    }).to_string())
                } else {
                    eprintln!("Multiple instances of \"seek\" expected one");
                    std::process::exit(3);
                }
            } else if arg.starts_with("skip") {
                if o_skip.is_none() {
                    o_skip = Some(arg.split("=").nth(1).unwrap_or_else(|| {
                        eprintln!("Expected skip=[INT], found: {arg}");
                        opts.usage(Self::BRIEF);
                        std::process::exit(2);
                    }).to_string())
                } else {
                    eprintln!("Multiple instances of \"skip\" expected one");
                    std::process::exit(3);
                }
            } else if arg.starts_with("status") {
                if status.is_none() {
                    status = Some(arg.split("=").nth(1).unwrap_or_else(|| {
                        eprintln!("Expected status=[INT], found: {arg}");
                        opts.usage(Self::BRIEF);
                        std::process::exit(2);
                    }).to_string())
                } else {
                    eprintln!("Multiple instances of \"status\" expected one");
                    std::process::exit(3);
                }
            }
        }

        if let Ok(Some(o)) = matches.opt_get("if") {
            eprintln!("set i_f to {o:?}");
            i_f = Some(o); // most of these require args. If they require one but dont have one opts.parse will return err
        }
        if let Ok(Some(o)) = matches.opt_get("of") {
            o_f = Some(o) // most of these require args. If they require one but dont have one opts.parse will return err
        }
        if let Ok(Some(o)) = matches.opt_get("count") {
            count = Some(o) // most of these require args. If they require one but dont have one opts.parse will return err
        }
        if let Ok(Some(o)) = matches.opt_get::<String>("bs") {
            o_bs = Some(o.clone()); // most of these require args. If they require one but dont have one opts.parse will return err
            i_bs = Some(o)
        } else {
            if let Ok(Some(o)) = matches.opt_get("ibs") {
                i_bs = Some(o) // most of these require args. If they require one but dont have one opts.parse will return err
            }
            if let Ok(Some(o)) = matches.opt_get("of") {
                o_bs = Some(o) // most of these require args. If they require one but dont have one opts.parse will return err
            }
        }
        if let Ok(Some(o)) = matches.opt_get("seek") {
            o_skip = Some(o) // most of these require args. If they require one but dont have one opts.parse will return err
        }
        if let Ok(Some(o)) = matches.opt_get("skip") {
            i_skip = Some(o) // most of these require args. If they require one but dont have one opts.parse will return err
        }
        if let Ok(Some(o)) = matches.opt_get("status") {
            status = Some(o) // most of these require args. If they require one but dont have one opts.parse will return err
        }

        let mut cfg = config::ParsedCfg::new();

        // load default config if allowed
        if !matches.opt_present("no-cfg") {
            let def = PathBuf::from(DEFAULT_CONFIG);
            if def.exists() {
                cfg.load(def)
            }
        }
        for i in matches.opt_strs("cfg") {
            cfg.load(i.into())
        }

        Self {
            o_f: o_f.map(|s| io::Target::Path(PathBuf::from(s))).unwrap_or(io::Target::StdOut),
            i_f: i_f.map(|s| io::Target::Path(PathBuf::from(s))).unwrap_or(io::Target::StdIn),
            i_bs: i_bs.map(|s| Self::parse_units(&*s)).unwrap_or(512),
            o_bs: o_bs.map(|s| Self::parse_units(&*s)).unwrap_or(512),
            count: count.map(|s| s.parse().unwrap_or_else( |_| {
                    eprintln!("Failed to parse {s}\nExpected integer");
                    std::process::exit(3); })
            ),
            o_skip: o_skip.map(|s| s.parse().unwrap_or_else( |_| {
                eprintln!("Failed to parse {s}\nExpected integer");
                std::process::exit(3); })
            ),
            i_skip: i_skip.map(|s| s.parse().unwrap_or_else( |_| {
                eprintln!("Failed to parse {s}\nExpected integer");
                std::process::exit(3); })
            ),
            status: Status::try_from(&*status.unwrap_or("none".to_string())).unwrap_or_else(|_| {
                eprintln!("Failed to parse argument for 'status'\nExpected 'none', 'noxfer' or 'progress'");
                std::process::exit(3); }),
            cfg,
        }
    }

    fn units_map() -> std::collections::HashMap<&'static str,usize> {
        let units: std::collections::HashMap<&str,usize> = std::collections::HashMap::from([
            ("kB",1000),
            ("K",1024),
            ("MB",1000usize.pow(2)),
            ("M",1024usize.pow(2)),
            ("GB",1000usize.pow(3)),
            ("G",1024usize.pow(3)),
            ("TB",1000usize.pow(4)),
            ("T",1024usize.pow(4)),
            ("PB",1000usize.pow(4)),
            ("P",1024usize.pow(4)),
        ]);
        units
    }

    fn parse_units(src: &str) -> usize {
        let mut src = src.to_string();
        let u = Self::units_map();
        for (k,v) in u {
            if src.ends_with(k) {
                src.truncate(src.len() - k.len());
                let val: usize = src.parse().unwrap_or_else(|_| {
                    eprintln!("Failed to parse {src}\nExpected integer and maybe a following unit eg. '64K'");
                    std::process::exit(3);
                });
                return val * v;
            }
        }
        return src.parse().unwrap_or_else(|_| {
            eprintln!("Failed to parse {src}\nExpected integer and maybe a following unit eg. '64K'");
            std::process::exit(3);
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Status {
    Default,
    NoXFer,
    Progress,
}

impl TryFrom<&str> for Status {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut s = value.to_string();
        s = s.to_lowercase();
        match &*s {
            s if s == "none" => Ok(Self::Default),
            s if s == "noxfer" => Ok(Self::NoXFer),
            s if s == "progress" => Ok(Self::Progress),
            _ => Err(()),
        }
    }
}
struct IoQueue {
    tx: std::sync::mpsc::Sender<Box<[u8]>>,
    pending: Vec<u8>,
    bs: usize,
}

impl IoQueue {
    fn new(obs: usize, tx: std::sync::mpsc::Sender<Box<[u8]>>) -> Self {

        let s = Self {
            tx,
            pending: Vec::new(),
            bs: obs,
        };
            s
    }

    fn push(&mut self, mut buff: Vec<u8>) {
        #[cfg(debug)]
        eprintln!("buff:    {buff:x?}");
        // normal branch, just send it.
        if buff.len() == self.bs {
            self.tx.send(buff.into_boxed_slice()).expect("Receiving thread closed channel");
            STATE.queued.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
            return
        }

        {
            #[cfg(debug)]
            eprintln!("Partial");
            self.pending.append(&mut buff);
            let ch = self.pending.chunks_exact(self.bs);
            let mut r = ch.remainder().to_vec();

            for i in ch {
                #[cfg(debug)]
                eprintln!("Sending: {i:x?}");
                STATE.queued.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.tx.send(i.to_vec().into_boxed_slice()).expect("Receiving thread closed channel");
            }

            self.pending.truncate(0);
            self.pending = r;

            #[cfg(debug)] {
                eprintln!("Remain:  {:x?}", self.pending);
                eprintln!("--------")
            }
        }
    }
}

impl Drop for IoQueue {
    fn drop(&mut self) {
        if self.pending.len() > 0 {

            self.tx.send(std::mem::replace(&mut self.pending,Vec::new()).into_boxed_slice()).expect("Receiving thread closed channel");
            STATE.queued.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[track_caller]
pub fn handle_err(e: std::io::Error, msg: &str, code: i32) -> ! {
    eprintln!("{} for {msg}",e.to_string());

    let c = std::panic::Location::caller();
    eprintln!("from {c}");
    std::process::exit(code)
}