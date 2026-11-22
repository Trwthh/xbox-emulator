use core::ffi::c_void;
use console_sandbox::crossvm;
use XVMAudioService::*;

const IID_ICLASSFACTORY: Guid = Guid { data1: 1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const STEP: &str = "TOMBOX_VERIFY_CROSSVM_STEP";
const CLSID_XVM_EVENT: Guid = Guid { data1: 0x148EC3BF, data2: 0x349C, data3: 0x4EAC, data4: [0xB3, 0x73, 0x9F, 0xD5, 0x7C, 0xB4, 0x6D, 0x88] };
const IID_XVM_EVENT_OPENER: Guid = Guid { data1: 0xC4E70434, data2: 0x407D, data3: 0x416B, data4: [0x94, 0xBE, 0x97, 0x17, 0xB7, 0x90, 0x65, 0xFB] };

#[link(name = "kernel32")]
extern "system" {
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
}

unsafe fn vslot<T: Copy>(obj: *mut c_void, slot: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(slot))
}

unsafe fn release(obj: *mut c_void) -> u32 {
    vslot::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj)
}

fn console() -> Option<String> {
    console_image::dump_dirs(std::path::Path::new(console_image::GAMES)).into_iter().map(|d| d.join("XVMAudioService.dll")).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
}

fn envelope(id: u32, object: u32) -> Vec<u8> {
    let mut e = vec![0u8; crossvm::ENVELOPE];
    e[8..12].copy_from_slice(&id.to_le_bytes());
    e[0xc..0x10].copy_from_slice(&object.to_le_bytes());
    e
}

fn send(e: &mut [u8]) -> i32 {
    let id = u32::from_le_bytes(e[8..12].try_into().unwrap());
    assert_eq!(unsafe { crossvm::call(e.len() as i32, e.as_mut_ptr()) }, 0);
    assert_eq!(u32::from_le_bytes(e[8..12].try_into().unwrap()), id, "the host echoes the call id");
    i32::from_le_bytes(e[4..8].try_into().unwrap())
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

#[test]
fn crossvm_audio_renders_the_guest_ring_through_wasapi() {
    if console().is_none() {
        eprintln!("no console XVMAudioService.dll, skipped");
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(&exe).args(["--exact", "inner", "--nocapture", "--test-threads=1"]).env(STEP, "render").env("TOMBOX_AUDIO_MUTE", "1").env("TOMBOX_AUDIO_CAPTURE", "1").output().unwrap();
    let text = format!("exit={:?}\n{}{}", out.status.code().map(|c| c as u32), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    println!("{text}");
    assert!(out.status.success() && (text.contains("RENDER OK") || text.contains("NO PC AUDIO ENDPOINT")), "{text}");
}

#[test]
fn inner() {
    let Ok(_) = std::env::var(STEP) else { return };
    let dir = std::env::temp_dir().join(format!("xvmaudio_crossvm_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    std::env::set_var("TOMBOX_XVMAUDIOSERVICE_CONSOLE", console().unwrap());
    let mut e = envelope(crossvm::GET_MIX_FORMAT, 0);
    let hr = send(&mut e);
    if hr < 0 {
        println!("NO PC AUDIO ENDPOINT: GetMixFormat {:#010x}", hr as u32);
        return;
    }
    let mix_len = 0x12 + u16::from_le_bytes([e[0x20], e[0x21]]) as usize;
    let mix = e[0x10..0x10 + mix_len].to_vec();
    let align = u16::from_le_bytes([mix[0xc], mix[0xd]]) as usize;
    let rate = u32_at(&mix, 4);
    println!("PC mix format {mix_len:#x} bytes, {rate} Hz, block {align}");
    assert!(mix_len <= 0x48 && align > 0);

    let mut e = envelope(crossvm::GET_DEVICE_PERIOD, 0);
    assert_eq!(send(&mut e), 0);
    let period = i64::from_le_bytes(e[0x10..0x18].try_into().unwrap());
    assert!(period > 0);

    let mut e = envelope(crossvm::CREATE_OBJECT, 0);
    e[0x10..0x14].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(send(&mut e), 0);
    let object = u32_at(&e, 0x1c);
    assert_ne!(object, 0);

    let mut e = envelope(crossvm::INITIALIZE, object);
    let a = 0x10;
    e[a + 8..a + 16].copy_from_slice(&1_000_000i64.to_le_bytes());
    e[a + 0x18..a + 0x18 + mix_len].copy_from_slice(&mix);
    assert_eq!(send(&mut e), 0, "Initialize with the PC mix format");
    let r = &e[a..];
    assert_eq!(u32_at(r, crossvm::INIT_MEMORY), crossvm::KIND_CROSS_VM_MEMORY);
    assert_eq!(u32_at(r, crossvm::INIT_EVENT1), crossvm::KIND_CROSS_VM_EVENT);
    let kind_ok = |k: u32| k & 0xffff_fffd == 0;
    assert!(kind_ok(u32_at(r, crossvm::INIT_EVENT1)) && kind_ok(u32_at(r, crossvm::INIT_EVENT2)) && kind_ok(u32_at(r, crossvm::INIT_EVENT3)), "the guest reply parser accepts event kinds 0 and 2 only");
    let index = u32_at(r, crossvm::INIT_MEMORY + 0x24);
    let (base, pool_index, frames, event_name, _) = crossvm::stream_info(object).expect("stream");
    assert_eq!(index, pool_index as u32);
    assert_eq!(base, console_sandbox::logan::pool() + index as usize * console_sandbox::logan::PAGE);

    let m = console_module();
    assert_ne!(m, 0);
    let theirs: unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(console_image::symbol(m, "DllGetClassObject").unwrap()) };
    let mut f = core::ptr::null_mut();
    assert_eq!(unsafe { theirs(&CLSID_LOGAN_BOUND, &IID_ICLASSFACTORY, &mut f) }, S_OK);
    let mut o = core::ptr::null_mut();
    assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(f, 3)(f, core::ptr::null_mut(), &IID_IUNKNOWN, &mut o) }, S_OK);
    let mut desc = [0u8; 0x30];
    desc[0..4].copy_from_slice(&r[crossvm::INIT_MEMORY..crossvm::INIT_MEMORY + 4]);
    desc[8..0x2c].copy_from_slice(&r[crossvm::INIT_MEMORY + 4..crossvm::INIT_MEMORY + 0x28]);
    assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *const u8) -> i32>(o, 3)(o, desc.as_ptr()) }, S_OK, "the console XvmBuffer accepts the memory descriptor from the Initialize reply");
    let mut size = 0u32;
    let mut va = core::ptr::null_mut::<c_void>();
    assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut c_void) -> i32>(o, 4)(o, &mut size, &mut va) }, S_OK);
    println!("guest XvmBuffer va {va:?} size {size:#x}, host block {base:#x}, {frames} frames, event {event_name}");
    assert_eq!(va as usize, base, "the guest maps the same pool run the host renders from");

    let mut ef = core::ptr::null_mut();
    assert_eq!(unsafe { theirs(&CLSID_XVM_EVENT, &IID_ICLASSFACTORY, &mut ef) }, S_OK);
    let mut eo = core::ptr::null_mut();
    assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(ef, 3)(ef, core::ptr::null_mut(), &IID_XVM_EVENT_OPENER, &mut eo) }, S_OK, "console event class 148ec3bf");
    let mut edesc = [0u8; 0x30];
    edesc[0..4].copy_from_slice(&r[crossvm::INIT_EVENT1..crossvm::INIT_EVENT1 + 4]);
    edesc[8..0x2e].copy_from_slice(&r[crossvm::INIT_EVENT1 + 4..crossvm::INIT_EVENT1 + 0x2a]);
    let mut guest_event: isize = 0;
    let ehr = unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *const u8, *mut isize) -> i32>(eo, 5)(eo, edesc.as_ptr(), &mut guest_event) };
    let opened = console_sandbox::vab::opened_names();
    println!("console GetHandle {:#010x} handle {guest_event:#x}, OpenEventW names {opened:?}", ehr as u32);
    assert_eq!(ehr, S_OK, "the console opens the host stream event from the Initialize descriptor");
    assert!(opened.contains(&format!("{event_name}{}", console_sandbox::vab::SUFFIX)), "the console derives the same name the host created");
    assert_ne!(guest_event, 0);
    let ev_obj = guest_event as *mut c_void;
    let guest_handle = unsafe { vslot::<unsafe extern "system" fn(*mut c_void) -> isize>(ev_obj, 3)(ev_obj) };
    println!("guest event object {ev_obj:?} -> handle {guest_handle:#x}");
    assert_ne!(guest_handle, 0);

    let cb = va as *mut u8;
    let rd = |o: usize| unsafe { (cb.add(o) as *const u32).read_unaligned() };
    assert_eq!(rd(crossvm::CB_MAGIC_AT), crossvm::CB_MAGIC);
    let c = crossvm::CB_CONFIG;
    let start = rd(c + crossvm::CFG_DATA_START) as usize;
    let end = rd(c + crossvm::CFG_DATA_END) as usize;
    let total = rd(c + crossvm::CFG_TOTAL) as usize;
    assert!(start < end && end <= total && total <= size as usize, "IsValidControlData rule start < end <= total within the mapped run");
    assert_eq!(rd(c) as usize, 0xc8 + mix_len - 0x12);
    assert_eq!((end - start) % align, 0);
    assert_eq!(unsafe { core::slice::from_raw_parts(cb.add(c + crossvm::CFG_FORMAT), mix_len) }, &mix[..]);

    let ring = end - start;
    let bytes = (ring / 2 / align) * align;
    let pattern: Vec<u8> = (0..bytes).map(|i| (i * 7 % 251) as u8).collect();
    unsafe {
        core::ptr::copy_nonoverlapping(pattern.as_ptr(), cb.add(start), bytes);
        let w = &*(cb.add(crossvm::CB_WRITE) as *const std::sync::atomic::AtomicI64);
        w.fetch_add(bytes as i64, std::sync::atomic::Ordering::SeqCst);
    }
    let mut e = envelope(crossvm::START, object);
    assert_eq!(send(&mut e), 0);
    let t = std::time::Instant::now();
    loop {
        let read = unsafe { (cb.add(crossvm::CB_READ) as *const i64).read_volatile() };
        if read >= bytes as i64 || t.elapsed().as_secs() > 5 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut e = envelope(crossvm::STOP, object);
    assert_eq!(send(&mut e), 0);
    assert_eq!(unsafe { WaitForSingleObject(guest_handle, 1000) }, 0, "the host signals the guest stream event after draining");
    let read = unsafe { (cb.add(crossvm::CB_READ) as *const i64).read_volatile() };
    let cap = crossvm::take_capture(object).expect("capture enabled");
    println!("host READ {read} of {bytes} written, captured {} bytes from the WASAPI render buffer", cap.len());
    assert_eq!(read, bytes as i64, "the host consumed exactly what the guest released");
    assert_eq!(&cap[..bytes], &pattern[..], "the bytes in the WASAPI render buffer are the guest's samples");

    unsafe { release(ev_obj) };
    unsafe { release(eo) };
    unsafe { release(ef) };
    unsafe { release(o) };
    unsafe { release(f) };
    let mut e = envelope(crossvm::RELEASE_OBJECT, object);
    assert_eq!(send(&mut e), 0);
    assert!(crossvm::stream_info(object).is_none());
    println!("RENDER OK");
}
