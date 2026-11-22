use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::{Mutex, OnceLock};
use xtfmsg::com::IID_XBTP_EXTENSION_METADATA;
use xtfmsg::sys::*;
use xtfremoterun::client::*;
use xtfremoterun::REMOTE_RUN_EXTENSION_ID;

extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 8;
const CONSOLE_DLLS: [&str; 2] = [r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\xtfremoterun.dll", r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\xtfremoterun.dll"];

type Factory = unsafe extern "system" fn(*const u16, *const GUID, *mut *mut c_void) -> HRESULT;
type Entry = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type Qi = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type Release = unsafe extern "system" fn(*mut c_void) -> u32;
type Count = unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT;
type RunEx = unsafe extern "system" fn(*mut c_void, *const u16, *const u16, u32, u32, *mut c_void, *mut u32) -> HRESULT;
type Run = unsafe extern "system" fn(*mut c_void, *const u16, *const u16, u32, u32, *mut c_void) -> HRESULT;

struct Console {
    factory: Factory,
    entry: Entry,
}

unsafe impl Send for Console {}
unsafe impl Sync for Console {}

static LOCK: Mutex<()> = Mutex::new(());

fn preload_current_xbtp() {
    let dir = std::env::temp_dir().join(format!("xtfremoterun-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    for a in exe.ancestors() {
        let Ok(rd) = std::fs::read_dir(a.join("build").join("xbtp")) else { continue };
        let newest = rd
            .flatten()
            .map(|e| e.path().join("out").join("xbtp.dll"))
            .filter(|p| p.is_file())
            .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        let src = newest.expect("no build/xbtp/*/out/xbtp.dll to preload");
        let copy = dir.join("xbtp.dll");
        std::fs::copy(&src, &copy).unwrap();
        assert!(xtfmsg::xbtp::preload(&copy), "{}", copy.display());
        return;
    }
    panic!("no build/xbtp directory above the test exe");
}

fn consoles() -> &'static Vec<(&'static str, Console)> {
    static C: OnceLock<Vec<(&'static str, Console)>> = OnceLock::new();
    C.get_or_init(|| {
        preload_current_xbtp();
        let mut v = Vec::new();
        for p in CONSOLE_DLLS {
            if !std::path::Path::new(p).is_file() {
                continue;
            }
            let h = unsafe { LoadLibraryExW(wide(p).as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
            if h.is_null() {
                continue;
            }
            unsafe {
                let f = GetProcAddress(h, b"XtfCreateRemoteRunClient\0".as_ptr());
                let e = GetProcAddress(h, 4usize as *const u8);
                assert!(!f.is_null() && !e.is_null(), "{p}: exports");
                v.push((p, Console { factory: core::mem::transmute::<*mut c_void, Factory>(f), entry: core::mem::transmute::<*mut c_void, Entry>(e) }));
            }
            break;
        }
        v
    })
}

unsafe fn slot<T: Copy>(o: *mut c_void, i: usize) -> T {
    let p = *(*(o as *const *const usize)).add(i);
    core::mem::transmute_copy(&p)
}

unsafe fn create(f: Factory, address: Option<&str>, iid: &GUID) -> (HRESULT, *mut c_void) {
    let a = address.map(wide);
    let mut out = 0x5555usize as *mut c_void;
    let hr = f(a.as_ref().map(|v| v.as_ptr()).unwrap_or(core::ptr::null()), iid, &mut out);
    (hr, out)
}

unsafe fn ours(address: Option<&str>, iid: &GUID) -> (HRESULT, *mut c_void) {
    let a = address.map(wide);
    let mut out = 0x5555usize as *mut c_void;
    let hr = XtfCreateRemoteRunClient(a.as_ref().map(|v| v.as_ptr()).unwrap_or(core::ptr::null()), iid, &mut out as *mut *mut c_void as *mut _);
    (hr, out)
}

unsafe fn release_all(o: *mut c_void) {
    let r = slot::<Release>(o, 2);
    while r(o) > 0 {}
}

const BAD: GUID = GUID::new(1, 2, 3, [4; 8]);

#[test]
fn factory_and_query_interface_match_console_dll() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some((path, c)) = consoles().first() else {
        eprintln!("console xtfremoterun.dll does not load on this PC; oracle skipped");
        return;
    };
    unsafe {
        for (addr, iid) in [(None, IID_XTF_REMOTE_RUN_CLIENT), (Some(""), IID_XTF_REMOTE_RUN_CLIENT2), (Some("1.2.3.4"), BAD)] {
            let (want, wout) = create(c.factory, addr, &iid);
            let (got, gout) = ours(addr, &iid);
            assert_eq!(got as u32, want as u32, "{path}: {addr:?}");
            assert_eq!(wout.is_null(), gout.is_null(), "{addr:?} out");
            if want >= 0 {
                release_all(wout);
                release_all(gout);
            }
        }
        for addr in [None, Some("127.0.0.1"), Some("1.2.3.4/title")] {
            let (want, console) = create(c.factory, addr, &IID_XTF_REMOTE_RUN_CLIENT);
            let (got, mine) = ours(addr, &IID_XTF_REMOTE_RUN_CLIENT);
            assert_eq!((got, want), (S_OK, S_OK), "{addr:?}");
            for iid in [IID_IUNKNOWN, IID_XTF_REMOTE_RUN_CLIENT_BASE, IID_XTF_REMOTE_RUN_CLIENT, IID_XTF_REMOTE_RUN_CLIENT2, BAD] {
                let mut a = 0x77usize as *mut c_void;
                let mut b = 0x77usize as *mut c_void;
                let hw = slot::<Qi>(console, 0)(console, &iid, &mut a);
                let hg = slot::<Qi>(mine, 0)(mine, &iid, &mut b);
                assert_eq!(hg, hw, "{addr:?} QI {iid:?}");
                assert_eq!(a == 0x77usize as *mut c_void, b == 0x77usize as *mut c_void, "{addr:?} QI out untouched");
                if hw >= 0 {
                    assert_eq!((a as usize).wrapping_sub(console as usize), (b as usize).wrapping_sub(mine as usize), "{addr:?} QI offset {iid:?}");
                    slot::<Release>(a, 2)(a);
                    slot::<Release>(b, 2)(b);
                }
            }
            release_all(console);
            release_all(mine);
        }
    }
}

unsafe fn run_ex_both(c: &Console, addr: Option<&str>, cmd: Option<&str>, flags: u32) -> ((HRESULT, u32), (HRESULT, u32)) {
    let mut res = Vec::new();
    for console in [true, false] {
        let (hr, o) = if console { create(c.factory, addr, &IID_XTF_REMOTE_RUN_CLIENT2) } else { ours(addr, &IID_XTF_REMOTE_RUN_CLIENT2) };
        assert_eq!(hr, S_OK);
        let cmd_w = cmd.map(wide);
        let mut code = 0x4242u32;
        let hr = slot::<RunEx>(o, 3)(o, cmd_w.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()), core::ptr::null(), flags, 0, null_mut(), &mut code);
        release_all(o);
        res.push((hr, code));
    }
    (res[0], res[1])
}

#[test]
fn run_ex_offline_paths_match_console_dll() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some((path, c)) = consoles().first() else {
        eprintln!("console xtfremoterun.dll does not load on this PC; oracle skipped");
        return;
    };
    unsafe {
        let cases: [(Option<&str>, Option<&str>, u32); 7] = [
            (Some("127.0.0.1"), None, 0),
            (Some("127.0.0.1"), Some(""), 0),
            (Some("127.0.0.1"), Some("x.exe"), 0x4000_0000),
            (Some("127.0.0.1"), Some("x.exe"), 0x8000_0000),
            (None, Some("x.exe"), 0),
            (Some("1.2.3.4/bogus"), Some("x.exe"), 0),
            (Some("/title"), Some("x.exe"), 0),
        ];
        for (addr, cmd, flags) in cases {
            let (want, got) = run_ex_both(c, addr, cmd, flags);
            assert_eq!(got.0 as u32, want.0 as u32, "{path}: {addr:?} {cmd:?} {flags:#x}");
            assert_eq!(got.1, want.1, "{addr:?} {cmd:?} {flags:#x} exit code");
        }
        let (hr, console) = create(c.factory, Some("1.2.3.4/bogus"), &IID_XTF_REMOTE_RUN_CLIENT);
        assert_eq!(hr, S_OK);
        let (_, mine) = ours(Some("1.2.3.4/bogus"), &IID_XTF_REMOTE_RUN_CLIENT);
        let x = wide("x.exe");
        let hw = slot::<Run>(console, 3)(console, x.as_ptr(), core::ptr::null(), 0, 0, null_mut());
        let hg = slot::<Run>(mine, 3)(mine, x.as_ptr(), core::ptr::null(), 0, 0, null_mut());
        assert_eq!(hg as u32, hw as u32, "Run");
        release_all(console);
        release_all(mine);
    }
}

#[test]
fn entry_point_matches_console_dll() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some((path, c)) = consoles().first() else {
        eprintln!("console xtfremoterun.dll does not load on this PC; oracle skipped");
        return;
    };
    unsafe {
        let mut out = 0x1234usize as *mut c_void;
        assert_eq!((c.entry)(&BAD, &IID_XBTP_EXTENSION_METADATA, &mut out), E_UNEXPECTED, "{path}");
        assert_eq!(xtfremoterun::XtfOrdinal_4(&BAD, &IID_XBTP_EXTENSION_METADATA, &mut out as *mut *mut c_void as *mut _), E_UNEXPECTED);
        assert_eq!((c.entry)(&REMOTE_RUN_EXTENSION_ID, &BAD, &mut out), E_UNEXPECTED);
        assert_eq!(xtfremoterun::XtfOrdinal_4(&REMOTE_RUN_EXTENSION_ID, &BAD, &mut out as *mut *mut c_void as *mut _), E_UNEXPECTED);
        assert_eq!(out, 0x1234usize as *mut c_void);
        let mut want: *mut c_void = null_mut();
        let mut got: *mut c_void = null_mut();
        assert_eq!((c.entry)(&REMOTE_RUN_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut want), S_OK);
        assert_eq!(xtfremoterun::XtfOrdinal_4(&REMOTE_RUN_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut got as *mut *mut c_void as *mut _), S_OK);
        assert!(!want.is_null() && !got.is_null());
        let (mut n, mut m) = (0u32, 0u32);
        assert_eq!(slot::<Count>(want, 3)(want, &mut n), S_OK);
        assert_eq!(slot::<Count>(got, 3)(got, &mut m), S_OK);
        assert_eq!((m, n), (1, 1));
        assert_eq!(xtfremoterun::HANDLERS.iter().map(|h| h.0).collect::<Vec<_>>(), [1]);
    }
}
