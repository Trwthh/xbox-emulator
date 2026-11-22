use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use crate::gcdcom::Guid;

const CLSID_MM_DEVICE_ENUMERATOR: Guid = Guid { data1: 0xBCDE0395, data2: 0xE52F, data3: 0x467C, data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E] };
const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid { data1: 0xA95664D2, data2: 0x9614, data3: 0x4F35, data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6] };
const IID_IAUDIO_CLIENT: Guid = Guid { data1: 0x1CB9AD4C, data2: 0xDBFA, data3: 0x4C32, data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2] };
const IID_IAUDIO_RENDER_CLIENT: Guid = Guid { data1: 0xF294ACFC, data2: 0x3146, data3: 0x4483, data4: [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2] };
const IID_ISIMPLE_AUDIO_VOLUME: Guid = Guid { data1: 0x87CE5498, data2: 0x68D6, data3: 0x44E5, data4: [0x92, 0x15, 0x6D, 0xA4, 0x7E, 0xF8, 0x83, 0xD8] };
const CLSCTX_ALL: u32 = 0x17;
const SHARED: u32 = 0;
const AUTOCONVERT: u32 = 0x8000_0000 | 0x0800_0000;
pub const MAX_FORMAT: usize = 0x48;

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(r: *mut c_void, f: u32) -> i32;
    fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
    fn CoTaskMemFree(p: *mut c_void);
}

#[link(name = "kernel32")]
extern "system" {
    fn SetEvent(h: usize) -> i32;
    fn Sleep(ms: u32);
}

unsafe fn slot<T: Copy>(obj: *mut c_void, n: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(n))
}

unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        slot::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj);
    }
}

pub fn format_len(f: &[u8]) -> usize {
    if f.len() < 0x12 {
        return 0;
    }
    let cb = u16::from_le_bytes([f[0x10], f[0x11]]) as usize;
    (0x12 + cb).min(MAX_FORMAT)
}

pub fn block_align(f: &[u8]) -> u32 {
    u16::from_le_bytes([f[0xc], f[0xd]]) as u32
}

pub fn sample_rate(f: &[u8]) -> u32 {
    u32::from_le_bytes(f[4..8].try_into().unwrap())
}

unsafe fn client() -> Result<*mut c_void, i32> {
    CoInitializeEx(core::ptr::null_mut(), 0);
    let mut en = core::ptr::null_mut();
    let hr = CoCreateInstance(&CLSID_MM_DEVICE_ENUMERATOR, core::ptr::null_mut(), CLSCTX_ALL, &IID_IMM_DEVICE_ENUMERATOR, &mut en);
    if hr < 0 {
        return Err(hr);
    }
    let mut dev = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32>(en, 4)(en, 0, 0, &mut dev);
    release(en);
    if hr < 0 {
        return Err(hr);
    }
    let mut c = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void, *mut *mut c_void) -> i32>(dev, 3)(dev, &IID_IAUDIO_CLIENT, CLSCTX_ALL, core::ptr::null(), &mut c);
    release(dev);
    if hr < 0 {
        return Err(hr);
    }
    Ok(c)
}

pub struct DeviceInfo {
    pub mix: Vec<u8>,
    pub default_period: i64,
    pub min_period: i64,
}

pub fn device_info() -> Result<DeviceInfo, i32> {
    unsafe {
        let c = client()?;
        let mut f: *mut u8 = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u8) -> i32>(c, 8)(c, &mut f);
        if hr < 0 {
            release(c);
            return Err(hr);
        }
        let head = core::slice::from_raw_parts(f, 0x12);
        let mix = core::slice::from_raw_parts(f, format_len(head)).to_vec();
        CoTaskMemFree(f as *mut c_void);
        let (mut d, mut m) = (0i64, 0i64);
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> i32>(c, 9)(c, &mut d, &mut m);
        release(c);
        if hr < 0 {
            return Err(hr);
        }
        Ok(DeviceInfo { mix, default_period: d, min_period: m })
    }
}

pub fn is_format_supported(fmt: &[u8]) -> i32 {
    unsafe {
        let c = match client() {
            Ok(c) => c,
            Err(hr) => return hr,
        };
        let mut closest: *mut u8 = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *const u8, *mut *mut u8) -> i32>(c, 7)(c, SHARED, fmt.as_ptr(), &mut closest);
        if !closest.is_null() {
            CoTaskMemFree(closest as *mut c_void);
        }
        release(c);
        hr
    }
}

pub struct Ring {
    pub base: usize,
    pub start: u32,
    pub end: u32,
    pub align: u32,
}

impl Ring {
    pub fn write_pos(&self) -> &AtomicI64 {
        unsafe { &*((self.base + 0x10) as *const AtomicI64) }
    }
    pub fn read_pos(&self) -> &AtomicI64 {
        unsafe { &*((self.base + 0x18) as *const AtomicI64) }
    }
    pub fn bytes(&self) -> u32 {
        self.end - self.start
    }
}

#[derive(Default)]
pub struct Shared {
    pub running: AtomicBool,
    pub quit: AtomicBool,
    pub gain: Mutex<Option<f32>>,
    pub capture: Mutex<Option<Vec<u8>>>,
    pub rendered: AtomicI64,
    pub signal: Mutex<Vec<isize>>,
}

pub struct Renderer {
    pub shared: Arc<Shared>,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn muted() -> bool {
    std::env::var("TOMBOX_AUDIO_MUTE").map(|v| v == "1").unwrap_or(false)
}

pub fn capturing() -> bool {
    std::env::var("TOMBOX_AUDIO_CAPTURE").map(|v| v == "1").unwrap_or(false)
}

pub fn open(fmt: Vec<u8>, duration: i64, ring: Ring) -> Result<(Renderer, u32), i32> {
    let shared = Arc::new(Shared::default());
    if capturing() {
        *shared.capture.lock().unwrap() = Some(Vec::new());
    }
    let (tx, rx) = std::sync::mpsc::channel::<Result<u32, i32>>();
    let s2 = shared.clone();
    let t = std::thread::spawn(move || unsafe { run(fmt, duration, ring, s2, tx) });
    match rx.recv() {
        Ok(Ok(frames)) => Ok((Renderer { shared, thread: Some(t) }, frames)),
        Ok(Err(hr)) => {
            let _ = t.join();
            Err(hr)
        }
        Err(_) => Err(-2147467259),
    }
}

unsafe fn run(fmt: Vec<u8>, duration: i64, ring: Ring, s: Arc<Shared>, tx: std::sync::mpsc::Sender<Result<u32, i32>>) {
    let c = match client() {
        Ok(c) => c,
        Err(hr) => {
            let _ = tx.send(Err(hr));
            return;
        }
    };
    let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, u32, i64, i64, *const u8, *const Guid) -> i32>(c, 3)(c, SHARED, AUTOCONVERT, duration, 0, fmt.as_ptr(), core::ptr::null());
    if hr < 0 {
        release(c);
        let _ = tx.send(Err(hr));
        return;
    }
    let mut frames = 0u32;
    slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c, 4)(c, &mut frames);
    let mut rc = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c, 14)(c, &IID_IAUDIO_RENDER_CLIENT, &mut rc);
    if hr < 0 {
        release(c);
        let _ = tx.send(Err(hr));
        return;
    }
    let mut vol = core::ptr::null_mut();
    slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c, 14)(c, &IID_ISIMPLE_AUDIO_VOLUME, &mut vol);
    if !vol.is_null() && muted() {
        slot::<unsafe extern "system" fn(*mut c_void, i32, *const Guid) -> i32>(vol, 5)(vol, 1, core::ptr::null());
    }
    let _ = tx.send(Ok(frames));
    let align = ring.align.max(1) as i64;
    let rate = sample_rate(&fmt).max(1) as i64;
    let tick = ((frames as i64 * 1000 / rate) / 4).clamp(1, 10) as u32;
    let mut playing = false;
    let mut applied_gain: Option<f32> = None;
    while !s.quit.load(Ordering::SeqCst) {
        let want = s.running.load(Ordering::SeqCst);
        if want != playing {
            slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c, if want { 10 } else { 11 })(c);
            playing = want;
        }
        let g = *s.gain.lock().unwrap();
        if g != applied_gain && !vol.is_null() {
            if let Some(v) = g {
                slot::<unsafe extern "system" fn(*mut c_void, f32, *const Guid) -> i32>(vol, 3)(vol, v.clamp(0.0, 1.0), core::ptr::null());
            }
            applied_gain = g;
        }
        if playing {
            pump(c, rc, frames, &ring, align, &s);
        }
        Sleep(tick);
    }
    if playing {
        slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c, 11)(c);
    }
    release(vol);
    release(rc);
    release(c);
}

unsafe fn pump(c: *mut c_void, rc: *mut c_void, frames: u32, ring: &Ring, align: i64, s: &Shared) {
    let mut pad = 0u32;
    if slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c, 6)(c, &mut pad) < 0 {
        return;
    }
    let w = ring.write_pos().load(Ordering::Acquire);
    let r = ring.read_pos().load(Ordering::Acquire);
    if w < r || r < 0 {
        return;
    }
    let avail = (w - r) / align;
    let n = avail.min(frames.saturating_sub(pad) as i64);
    if n <= 0 {
        return;
    }
    let mut dst: *mut u8 = core::ptr::null_mut();
    if slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut u8) -> i32>(rc, 3)(rc, n as u32, &mut dst) < 0 || dst.is_null() {
        return;
    }
    let total = (n * align) as usize;
    let size = ring.bytes() as usize;
    let mut done = 0usize;
    while done < total {
        let off = ((r as usize + done) % size) as usize;
        let chunk = (size - off).min(total - done);
        core::ptr::copy_nonoverlapping((ring.base + ring.start as usize + off) as *const u8, dst.add(done), chunk);
        done += chunk;
    }
    if let Some(cap) = s.capture.lock().unwrap().as_mut() {
        cap.extend_from_slice(core::slice::from_raw_parts(dst, total));
    }
    slot::<unsafe extern "system" fn(*mut c_void, u32, u32) -> i32>(rc, 4)(rc, n as u32, 0);
    ring.read_pos().store(r + total as i64, Ordering::Release);
    s.rendered.fetch_add(total as i64, Ordering::SeqCst);
    for &e in s.signal.lock().unwrap().iter() {
        SetEvent(e as usize);
    }
}
