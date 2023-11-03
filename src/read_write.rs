use std::io::{Read, Seek, SeekFrom, Write};
use crate::io::IoMode;
use crate::Options;



fn new_buff(len: usize) -> Vec<u8> {
    let mut  v = Vec::new();
    v.resize(len,0);
    v
}

pub fn dd_read(opts: Options, tx: std::sync::mpsc::Sender<Box<[u8]>>) {
    let mut f = opts.i_f.open(IoMode::Read,&opts);
    let mut queue = super::IoQueue::new(opts.o_bs,tx);

    if let Some(skip) = opts.i_skip {
        f.seek(SeekFrom::Start(skip as u64 * opts.i_bs as u64)).unwrap_or_else(|e| super::handle_err(e,&format!("in file {:?}",opts.i_f),0x20));
    }

    let max_queued_len = 1; // todo change to x bytes

    let mut b = new_buff(opts.i_bs);
    for _ in 0..opts.count.unwrap_or(usize::MAX) {
        while super::STATE.queued.load(std::sync::atomic::Ordering::Relaxed) >= max_queued_len {
            std::thread::sleep(std::time::Duration::from_millis(10)); //todo handle better
        }

        let r_len = f.read(&mut b).unwrap_or_else(|e| super::handle_err(e,&format!("in file {}",opts.i_f),0x21));
        b.truncate(r_len);

        //eprintln!("r: {b:x?}");

        queue.push(core::mem::replace(&mut b,new_buff(opts.i_bs)));

        // incomplete read is exit condition
        if r_len < opts.i_bs {
            super::STATE.read_extra.store(true,std::sync::atomic::Ordering::Relaxed);
            break
        }

        super::STATE.read_blk.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
    }
}



pub fn dd_write(opts: Options, rx: std::sync::mpsc::Receiver<Box<[u8]>>) {
    let mut f = opts.o_f.open(IoMode::Write,&opts);
    if let Some(skip) = opts.o_skip {
        f.seek(SeekFrom::Start(skip as u64 * opts.o_bs as u64)).unwrap_or_else(|e| super::handle_err(e,&format!("in file {:?}",opts.o_f), 0x20));
    }


    while let Ok(blk) = rx.recv() {
        super::STATE.queued.fetch_sub(1,std::sync::atomic::Ordering::Relaxed);
        let len: usize = blk.len();
        let rc = f.write(&*blk).unwrap_or_else(|e| super::handle_err(e,&format!("in file {:?}",opts.o_f), 0x21));

        //eprintln!("w: {:x?}",blk);

        if rc < opts.o_bs || len < opts.o_bs {
            super::STATE.write_extra.store(true,std::sync::atomic::Ordering::Relaxed);
            break
        }
        super::STATE.write_blk.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
    }
}