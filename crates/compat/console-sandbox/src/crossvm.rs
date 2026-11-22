use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::gcdcom::Guid;
use crate::wasapi;

pub const ENVELOPE: usize = 0x810;
pub const PAYLOAD: usize = 0x10;
pub const CREATE_OBJECT: u32 = 0x01;
pub const GET_MIX_FORMAT: u32 = 0x02;
pub const GET_DEVICE_PERIOD: u32 = 0x03;
pub const INITIALIZE: u32 = 0x04;
pub const IS_FORMAT_SUPPORTED: u32 = 0x05;
pub const GET_STREAM_LATENCY: u32 = 0x06;
pub const START: u32 = 0x07;
pub const STOP: u32 = 0x08;
pub const GET_SIMPLE_VOLUME: u32 = 0x15;
pub const SET_SIMPLE_VOLUME: u32 = 0x16;
pub const GET_SIMPLE_MUTE: u32 = 0x17;
pub const SET_SIMPLE_MUTE: u32 = 0x18;
pub const RELEASE_OBJECT: u32 = 0x2e;
pub const DEVICE_GET_STATE: u32 = 0x28;
pub const ENUMERATOR_RESET: u32 = 0x23;
pub const DEVICE_GET_ID: u32 = 0x27;
pub const DEVICE_ID_CHARS: usize = 0x82;
pub const DEVICE_ID_COUNT: usize = 0x104;
pub const EVENT_KIND_SRA: u32 = 1;
pub const EVENT_KIND_GAMECORE: u32 = 2;
pub const SET_ENDPOINT_VOLUME_SCALAR: u32 = 0x53;
pub const GET_ENDPOINT_VOLUME_SCALAR: u32 = 0x55;
pub const SET_ENDPOINT_MUTE: u32 = 0x5a;
pub const GET_ENDPOINT_MUTE: u32 = 0x5b;
pub const INIT_STRUCT: usize = 0x534;
pub const MAX_OBJECT_TYPE: u32 = 0x17;

pub const CB_MAGIC: u32 = 0x4550_4344;
pub const CB_WRITE: usize = 0x10;
pub const CB_READ: usize = 0x18;
pub const CB_META: usize = 0xb4;
pub const CB_MAGIC_AT: usize = 0xc0;
pub const CB_CONFIG: usize = 0xc8;
pub const CFG_FIXED: u32 = 0xc8;
pub const CFG_ENTRIES: usize = 0x84;
pub const CFG_DURATION: usize = 0x88;
pub const CFG_LOOPBACK: usize = 0x98;
pub const CFG_QUEUE: usize = 0x9c;
pub const CFG_DATA_START: usize = 0xa0;
pub const CFG_DATA_END: usize = 0xa4;
pub const CFG_TOTAL: usize = 0xa8;
pub const CFG_FORMAT: usize = 0xb4;
pub const QUEUE_ENTRY: u32 = 0x40;
pub const MIN_QUEUE_ENTRIES: u32 = 0x19;

pub const INIT_FORMAT: usize = 0x18;
pub const INIT_ENDPOINT_CLSID: usize = 0x70;
pub const INIT_FRAMES: usize = 0x80;
pub const INIT_PERIOD: usize = 0x84;
pub const INIT_LATENCY: usize = 0x8c;
pub const INIT_EVENT1: usize = 0x94;
pub const INIT_EVENT2: usize = 0x1b8;
pub const INIT_EVENT3: usize = 0x2dc;
pub const INIT_MEMORY: usize = 0x404;
pub const KIND_NONE: u32 = 0;
pub const KIND_CROSS_VM_EVENT: u32 = 2;
pub const KIND_CROSS_VM_MEMORY: u32 = 3;

pub const CLSID_CLIENT_OUTPUT_ENDPOINT: Guid = Guid { data1: 0xCD773740, data2: 0xB187, data3: 0x4974, data4: [0xA1, 0xD5, 0xE0, 0xFF, 0x91, 0x37, 0x22, 0x77] };
pub const CLSID_XVM_EVENT: Guid = Guid { data1: 0x148EC3BF, data2: 0x349C, data3: 0x4EAC, data4: [0xB3, 0x73, 0x9F, 0xD5, 0x7C, 0xB4, 0x6D, 0x88] };
pub const GUID_XVM_EVENT: Guid = Guid { data1: 0xA963C7E3, data2: 0x8A0A, data3: 0x45FA, data4: [0xBD, 0x23, 0xAA, 0x51, 0xE4, 0xCF, 0x85, 0x1E] };
pub const CLSID_XVM_BUFFER: Guid = Guid { data1: 0xE782CED1, data2: 0x3B5D, data3: 0x43AA, data4: [0xA4, 0x5D, 0x82, 0x85, 0x2B, 0x57, 0xD8, 0x4C] };
pub const GUID_XVM_BUFFER: Guid = Guid { data1: 0x00AA2A8A, data2: 0xACB6, data3: 0x4F3D, data4: [0x9A, 0x7E, 0xBD, 0xE7, 0x12, 0x7B, 0x43, 0x87] };

const S_OK: i32 = 0;
const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;
const AUDCLNT_E_NOT_INITIALIZED: i32 = 0x8889_0001u32 as i32;
const AUDCLNT_E_ALREADY_INITIALIZED: i32 = 0x8889_0002u32 as i32;
const HNS: i64 = 10_000_000;

pub struct Stream {
    pub pool_index: u16,
    pub base: usize,
    pub format: Vec<u8>,
    pub frames: u32,
    pub event: isize,
    pub event_name: String,
    pub renderer: wasapi::Renderer,
}

#[derive(Default)]
struct Object {
    stream: Option<Stream>,
    gain: f32,
    mute: bool,
}

#[derive(Default)]
struct State {
    objects: HashMap<u32, Object>,
    next: u32,
    next_event: u32,
}

fn state() -> MutexGuard<'static, State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { next: 1, ..Default::default() })).lock().unwrap_or_else(|e| e.into_inner())
}

fn guid_bytes(g: &Guid) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&g.data1.to_le_bytes());
    b[4..6].copy_from_slice(&g.data2.to_le_bytes());
    b[6..8].copy_from_slice(&g.data3.to_le_bytes());
    b[8..16].copy_from_slice(&g.data4);
    b
}

fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

pub fn event_name(index: u16) -> String {
    format!("{}{index}", crate::vab::SRA_BASE)
}

pub struct Layout {
    pub frames: u32,
    pub bytes: u32,
    pub entries: u32,
    pub queue: u32,
    pub start: u32,
    pub end: u32,
    pub total: u32,
}

pub fn layout(format: &[u8], frames: u32, period_frames: u32) -> Layout {
    let cb = (wasapi::format_len(format) - 0x12) as u32;
    let bytes = frames * wasapi::block_align(format);
    let entries = (frames / period_frames.max(1) + 3).max(MIN_QUEUE_ENTRIES);
    let size = CFG_FIXED + cb;
    let queue = align_up(CB_CONFIG as u32 + size, 0x80);
    let start = align_up(queue + entries * QUEUE_ENTRY, 0x400);
    let end = start + bytes;
    Layout { frames, bytes, entries, queue, start, end, total: end }
}

pub unsafe fn write_control_block(base: usize, format: &[u8], duration: i64, l: &Layout) {
    let p = base as *mut u8;
    core::ptr::write_bytes(p, 0, l.start as usize);
    let w32 = |o: usize, v: u32| (p.add(o) as *mut u32).write_unaligned(v);
    w32(CB_META, u32::MAX);
    w32(CB_MAGIC_AT, CB_MAGIC);
    let c = CB_CONFIG;
    let flen = wasapi::format_len(format);
    w32(c, CFG_FIXED + (flen as u32 - 0x12));
    w32(c + CFG_ENTRIES, l.entries);
    (p.add(c + CFG_DURATION) as *mut i64).write_unaligned(duration);
    w32(c + CFG_LOOPBACK, u32::MAX);
    w32(c + CFG_QUEUE, l.queue);
    w32(c + CFG_DATA_START, l.start);
    w32(c + CFG_DATA_END, l.end);
    w32(c + CFG_TOTAL, l.total);
    core::ptr::copy_nonoverlapping(format.as_ptr(), p.add(c + CFG_FORMAT), flen);
}

unsafe fn rd<T: Copy>(p: *const u8, o: usize) -> T {
    (p.add(o) as *const T).read_unaligned()
}

unsafe fn wr<T: Copy>(p: *mut u8, o: usize, v: T) {
    (p.add(o) as *mut T).write_unaligned(v)
}

pub unsafe fn call(cb: i32, env: *mut u8) -> i32 {
    if env.is_null() || (cb as usize) < PAYLOAD {
        return E_INVALIDARG;
    }
    let id: u32 = rd(env, 8);
    let object: u32 = rd(env, 0xc);
    let a = env.add(PAYLOAD);
    let room = cb as usize - PAYLOAD;
    let hr = dispatch(id, object, a, room);
    wr(env, 4, hr);
    wr(env, 8, id);
    compat_common::record_raw("crossvm-audio", "call", &format!("id {id:#x} object {object:#x} -> {:#010x}", hr as u32));
    S_OK
}

unsafe fn dispatch(id: u32, object: u32, a: *mut u8, room: usize) -> i32 {
    match id {
        CREATE_OBJECT => {
            if room < 0x10 {
                return E_INVALIDARG;
            }
            let kind: u32 = rd(a, 0);
            let flag: u32 = rd(a, 4);
            let mut s = state();
            let n = s.next;
            s.next += 1;
            s.objects.insert(n, Object { gain: 1.0, ..Default::default() });
            compat_common::record_raw("crossvm-audio", "CreateObject", &format!("type {kind:#x} -> object {n:#x}"));
            core::ptr::write_bytes(a, 0, 0x10);
            wr(a, 0, if kind < MAX_OBJECT_TYPE { kind } else { 0 });
            wr(a, 4, if flag <= 1 { flag } else { 0 });
            wr(a, 0xc, n);
            S_OK
        }
        RELEASE_OBJECT => {
            let o = state().objects.remove(&object);
            if let Some(Object { stream: Some(st), .. }) = o {
                drop(st.renderer);
                crate::logan::pool_release(st.pool_index as usize);
            }
            S_OK
        }
        GET_MIX_FORMAT => match wasapi::device_info() {
            Ok(d) => {
                core::ptr::write_bytes(a, 0, wasapi::MAX_FORMAT.min(room));
                core::ptr::copy_nonoverlapping(d.mix.as_ptr(), a, d.mix.len());
                S_OK
            }
            Err(hr) => hr,
        },
        GET_DEVICE_PERIOD => match wasapi::device_info() {
            Ok(d) => {
                wr(a, 0, d.default_period);
                wr(a, 8, d.min_period);
                S_OK
            }
            Err(hr) => hr,
        },
        GET_STREAM_LATENCY => match wasapi::device_info() {
            Ok(d) => {
                wr(a, 0, d.default_period);
                S_OK
            }
            Err(hr) => hr,
        },
        IS_FORMAT_SUPPORTED => {
            let f = core::slice::from_raw_parts(a.add(6), 0x12);
            let len = wasapi::format_len(f);
            if len < 0x12 {
                return E_INVALIDARG;
            }
            wasapi::is_format_supported(core::slice::from_raw_parts(a.add(6), len))
        }
        INITIALIZE => initialize(object, a, room),
        START | STOP => {
            let s = state();
            match s.objects.get(&object).and_then(|o| o.stream.as_ref()) {
                Some(st) => {
                    if id == START {
                        let mut sig = st.renderer.shared.signal.lock().unwrap();
                        sig.clear();
                        sig.push(st.event);
                        sig.extend(crate::vab::sra_events());
                    }
                    st.renderer.shared.running.store(id == START, Ordering::SeqCst);
                    S_OK
                }
                None => AUDCLNT_E_NOT_INITIALIZED,
            }
        }
        SET_SIMPLE_VOLUME | SET_ENDPOINT_VOLUME_SCALAR => {
            let v: f32 = rd(a, 0x10);
            if !(0.0..=1.0).contains(&v) {
                return E_INVALIDARG;
            }
            let mut s = state();
            let o = s.objects.entry(object).or_default();
            o.gain = v;
            apply(o);
            S_OK
        }
        SET_SIMPLE_MUTE | SET_ENDPOINT_MUTE => {
            let m: u8 = rd(a, 0x10);
            let mut s = state();
            let o = s.objects.entry(object).or_default();
            o.mute = m != 0;
            apply(o);
            S_OK
        }
        GET_SIMPLE_VOLUME | GET_ENDPOINT_VOLUME_SCALAR => {
            let g = state().objects.get(&object).map(|o| o.gain).unwrap_or(1.0);
            wr(a, 0, g);
            S_OK
        }
        GET_SIMPLE_MUTE | GET_ENDPOINT_MUTE => {
            let m = state().objects.get(&object).map(|o| o.mute).unwrap_or(false);
            wr(a, 0, m as u32);
            S_OK
        }
        ENUMERATOR_RESET => S_OK,
        DEVICE_GET_ID => match wasapi::device_id() {
            Ok(id) => {
                let w: Vec<u16> = id.encode_utf16().take(DEVICE_ID_CHARS - 1).chain(Some(0)).collect();
                core::ptr::write_bytes(a, 0, DEVICE_ID_COUNT + 8);
                core::ptr::copy_nonoverlapping(w.as_ptr() as *const u8, a, w.len() * 2);
                wr(a, DEVICE_ID_COUNT, (w.len() * 2) as u64);
                S_OK
            }
            Err(hr) => hr,
        },
        DEVICE_GET_STATE => {
            wr(a, 0, if wasapi::device_info().is_ok() { 1u32 } else { 4u32 });
            S_OK
        }
        _ => {
            compat_common::record_raw("crossvm-audio", "unserved", &format!("call id {id:#x} has no decoded argument layout; E_NOTIMPL"));
            E_NOTIMPL
        }
    }
}

fn apply(o: &Object) {
    if let Some(st) = &o.stream {
        *st.renderer.shared.gain.lock().unwrap() = Some(if o.mute { 0.0 } else { o.gain });
    }
}

unsafe fn initialize(object: u32, a: *mut u8, room: usize) -> i32 {
    if room < INIT_STRUCT {
        return E_INVALIDARG;
    }
    if state().objects.get(&object).map(|o| o.stream.is_some()).unwrap_or(false) {
        return AUDCLNT_E_ALREADY_INITIALIZED;
    }
    let duration: i64 = rd(a, 8);
    let head = core::slice::from_raw_parts(a.add(INIT_FORMAT), 0x12);
    let flen = wasapi::format_len(head);
    let format = core::slice::from_raw_parts(a.add(INIT_FORMAT), flen).to_vec();
    let align = wasapi::block_align(&format);
    let rate = wasapi::sample_rate(&format);
    if align == 0 || rate == 0 {
        return E_INVALIDARG;
    }
    let dev = match wasapi::device_info() {
        Ok(d) => d,
        Err(hr) => return hr,
    };
    let period_frames = (dev.default_period * rate as i64 / HNS).max(1) as u32;
    let frames = ((duration.max(dev.default_period * 2) * rate as i64 + HNS - 1) / HNS) as u32;
    let l = layout(&format, frames, period_frames);
    let pages = (l.total as usize).div_ceil(crate::logan::PAGE);
    let Some(index) = crate::logan::pool_alloc(pages) else {
        return E_OUTOFMEMORY;
    };
    let base = crate::logan::pool() + index as usize * crate::logan::PAGE;
    let hns = frames as i64 * HNS / rate as i64;
    write_control_block(base, &format, hns, &l);
    let ring = wasapi::Ring { base, start: l.start, end: l.end, align };
    let (renderer, device_frames) = match wasapi::open(format.clone(), hns, ring) {
        Ok(r) => r,
        Err(hr) => {
            crate::logan::pool_release(index as usize);
            return hr;
        }
    };
    let event_id = {
        let mut s = state();
        s.next_event += 1;
        s.next_event
    };
    let name = event_name(event_id as u16);
    let event = match crate::vab::create_event(&name) {
        Ok(h) => h,
        Err(e) => {
            crate::logan::pool_release(index as usize);
            return (0x8007_0000 | (e & 0xffff)) as i32;
        }
    };
    core::ptr::write_bytes(a.add(INIT_ENDPOINT_CLSID), 0, INIT_STRUCT - INIT_ENDPOINT_CLSID);
    core::ptr::copy_nonoverlapping(format.as_ptr(), a.add(INIT_FORMAT), flen);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_CLIENT_OUTPUT_ENDPOINT).as_ptr(), a.add(INIT_ENDPOINT_CLSID), 16);
    wr(a, INIT_FRAMES, frames);
    wr(a, INIT_PERIOD, dev.default_period);
    wr(a, INIT_LATENCY, dev.default_period);
    write_event_descriptor(a.add(INIT_EVENT1), EVENT_KIND_SRA, event_id as u16);
    wr(a, INIT_EVENT2, KIND_NONE);
    wr(a, INIT_EVENT3, KIND_NONE);
    wr(a, INIT_MEMORY, KIND_CROSS_VM_MEMORY);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_XVM_BUFFER).as_ptr(), a.add(INIT_MEMORY + 4), 16);
    core::ptr::copy_nonoverlapping(guid_bytes(&GUID_XVM_BUFFER).as_ptr(), a.add(INIT_MEMORY + 0x14), 16);
    wr(a, INIT_MEMORY + 0x24, index as u32);
    compat_common::record_raw("crossvm-audio", "Initialize", &format!("object {object:#x}: {frames} frames ({} bytes) at pool page {index}, WASAPI buffer {device_frames} frames, event {name}", l.bytes));
    let mut s = state();
    let o = s.objects.entry(object).or_insert_with(|| Object { gain: 1.0, ..Default::default() });
    o.stream = Some(Stream { pool_index: index, base, format, frames, event, event_name: name, renderer });
    S_OK
}

pub unsafe fn write_event_descriptor(p: *mut u8, kind: u32, index: u16) {
    wr(p, 0, KIND_CROSS_VM_EVENT);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_XVM_EVENT).as_ptr(), p.add(4), 16);
    core::ptr::copy_nonoverlapping(guid_bytes(&GUID_XVM_EVENT).as_ptr(), p.add(0x14), 16);
    wr(p, 0x24, kind);
    wr(p, 0x28, index);
}

pub fn stream_info(object: u32) -> Option<(usize, u16, u32, String, i64)> {
    let s = state();
    let st = s.objects.get(&object)?.stream.as_ref()?;
    Some((st.base, st.pool_index, st.frames, st.event_name.clone(), st.renderer.shared.rendered.load(Ordering::SeqCst)))
}

pub fn take_capture(object: u32) -> Option<Vec<u8>> {
    let s = state();
    let st = s.objects.get(&object)?.stream.as_ref()?;
    let mut c = st.renderer.shared.capture.lock().unwrap();
    c.as_mut().map(std::mem::take)
}

pub fn objects() -> usize {
    state().objects.len()
}
