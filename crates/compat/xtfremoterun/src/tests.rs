use crate::console::*;
use crate::launch::*;
use crate::server::*;
use crate::win::*;
use crate::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};
use xtfmsg::com::{self, Obj, IID_XBTP_EXTENSION_METADATA};
use xtfmsg::fixture::{self, end_json, end_message, load_xbtp, message_with_json, push_inbound, push_inbound_error};
use xtfmsg::sys::*;

pub static LOCK: Mutex<()> = Mutex::new(());
static INIT: Once = Once::new();

pub const CHILD_ENV: &str = "XTFREMOTERUN_CHILD";
pub const CHILD_IMAGE: &str = "D:\\DevelopmentFiles\\xtfremoterun_child.exe";
pub const TEST_LINK_PORT: u32 = 47914;

pub fn root() -> PathBuf {
    std::env::temp_dir().join(format!("xtfremoterun-test-{}", std::process::id()))
}

pub fn guard() -> MutexGuard<'static, ()> {
    INIT.call_once(|| {
        let r = root();
        let _ = std::fs::remove_dir_all(&r);
        std::fs::create_dir_all(r.join("console").join("fs").join("C").join("Windows").join("System32")).unwrap();
        std::env::set_var("TOMBOX_GDK_ROOT", &r);
        std::env::set_var(CHILD_ENV, "1");
        crate::client::TEST_LINK_PORT.store(TEST_LINK_PORT, std::sync::atomic::Ordering::SeqCst);
        register_image(CHILD_IMAGE, std::env::current_exe().unwrap());
        preload_current_xbtp(&r);
        load_xbtp();
    });
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn preload_current_xbtp(dir: &std::path::Path) {
    let exe = std::env::current_exe().unwrap();
    for a in exe.ancestors() {
        let b = a.join("build").join("xbtp");
        let Ok(rd) = std::fs::read_dir(&b) else { continue };
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

pub fn child_cmd(helper: &str) -> String {
    format!("{CHILD_IMAGE} tests::{helper} --exact --ignored --nocapture --test-threads=1 -q")
}

fn child_enabled() -> bool {
    std::env::var(CHILD_ENV).map(|v| v == "1").unwrap_or(false) && std::env::args().any(|a| a == "--ignored")
}

#[test]
#[ignore]
fn child_streams_main() {
    if !child_enabled() {
        return;
    }
    let mut o = std::io::stdout();
    o.write_all(b"child out\n").unwrap();
    o.flush().unwrap();
    let mut e = std::io::stderr();
    e.write_all(b"child err\n").unwrap();
    e.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    std::process::exit(3);
}

#[test]
#[ignore]
fn child_echo_main() {
    if !child_enabled() {
        return;
    }
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match std::io::stdin().read(&mut b) {
            Ok(1) if b[0] == b'\n' => break,
            Ok(1) => buf.push(b[0]),
            _ => break,
        }
    }
    let mut o = std::io::stdout();
    o.write_all(format!("got:{}\n", String::from_utf8_lossy(&buf)).as_bytes()).unwrap();
    o.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    std::process::exit(0);
}

#[test]
#[ignore]
fn child_wait_main() {
    if !child_enabled() {
        return;
    }
    std::thread::sleep(Duration::from_secs(60));
    std::process::exit(0);
}

#[test]
#[ignore]
fn child_marker_main() {
    if !child_enabled() {
        return;
    }
    std::fs::write("xtfremoterun_marker.txt", std::process::id().to_string()).unwrap();
    std::process::exit(5);
}

pub struct Jobs {
    pub names: Vec<&'static str>,
    pub jobs: Vec<OwnedHandle>,
}

#[repr(C)]
struct ExtendedLimit {
    basic: [u64; 8],
    io: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process: usize,
    peak_job: usize,
}

pub unsafe fn kill_on_close_job() -> OwnedHandle {
    let job = CreateJobObjectW(core::ptr::null(), core::ptr::null());
    assert!(!job.is_null());
    let mut info: ExtendedLimit = core::mem::zeroed();
    let flags = &mut info.basic[2] as *mut u64 as *mut u32;
    *flags = 0x2000;
    assert_ne!(SetInformationJobObject(job, 9, &info as *const ExtendedLimit as *const c_void, core::mem::size_of::<ExtendedLimit>() as u32), 0);
    OwnedHandle(job)
}

impl Jobs {
    pub fn new(names: &[&'static str]) -> Jobs {
        unsafe {
            let mut jobs = Vec::new();
            for n in names {
                let j = kill_on_close_job();
                register_job(n, j.get());
                jobs.push(j);
            }
            Jobs { names: names.to_vec(), jobs }
        }
    }
    pub fn title() -> Jobs {
        Jobs::new(&[NT_TITLE_JOB, HOST_TITLE_JOB])
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        for n in &self.names {
            unregister_job(n);
        }
    }
}

pub fn wait_for<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn request(flags: u32, period: u32, cmd: &str, wd: &str, stderr: bool) -> String {
    let mut s = format!("{{\"dwFlags\":{flags},\"dwPeriod\":{period},\"CommandLine\":\"{}\",\"WorkingDirectory\":\"{}\"", esc(cmd), esc(wd));
    if stderr {
        s.push_str(",\"Run_Error_Output\":1");
    }
    s.push('}');
    s
}

pub struct HandlerRun {
    pub end: Option<String>,
    pub sent: Vec<String>,
    pub flags: Vec<u32>,
    pub elapsed: Duration,
}

pub unsafe fn run_with(req: &str, inbound: &[Result<&str, HRESULT>]) -> HandlerRun {
    let s = fixture::session();
    for m in inbound {
        match m {
            Ok(j) => push_inbound(s, j),
            Err(hr) => push_inbound_error(s, *hr),
        }
    }
    let msg = message_with_json(req);
    let reply = end_message();
    let t = Instant::now();
    assert_eq!(run_handler(s as usize, msg.get() as usize, reply.get() as usize), S_OK);
    let elapsed = t.elapsed();
    let end = end_json(reply.get());
    let sent = (*s).sent.lock().unwrap().clone();
    let flags = (*s).receive_flags.lock().unwrap().clone();
    com::release(fixture::as_obj(s));
    HandlerRun { end, sent, flags, elapsed }
}

fn error_body(rc: u32, lines: &[u32]) -> String {
    let errs: Vec<String> = lines.iter().map(|l| format!("\"Error 0x{rc:X} File xbox\\\\devex\\\\xtf\\\\dll\\\\remoterun\\\\run.cpp : {l}\"")).collect();
    format!("{{\"XtfOperationError\":{{\"Checkpoint\":0,\"Errors\":[{}],\"ReturnCode\":{}}}}}", errs.join(","), rc as i32)
}

fn joined(sent: &[String], id: u32, key: &str) -> String {
    let mut out = String::new();
    for s in sent {
        if s.contains(&format!("\"MsgId\":{id}")) {
            let k = format!("\"{key}\":\"");
            if let Some(i) = s.find(&k) {
                let rest = &s[i + k.len()..];
                let end = rest.find("\"").unwrap();
                out.push_str(&rest[..end].replace("\\n", "\n").replace("\\r", "\r"));
            }
        }
    }
    out
}

#[test]
fn export_table_matches_console_dll() {
    let _g = guard();
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/exports.txt")).unwrap();
    xtfcommon::exports::check_against_titles("xtfremoterun.dll", &text);
    for e in xtfcommon::exports::parse(&text) {
        if e.status != "missing" {
            assert!(xtfcommon::exports::source_defines(concat!(env!("CARGO_MANIFEST_DIR"), "/src"), &e), "{}", e.name);
        }
    }
}

#[test]
fn entry_point_checks_ids_and_registers_run_handler() {
    let _g = guard();
    unsafe {
        let mut out: Obj = null_mut();
        let other = GUID::new(1, 2, 3, [4; 8]);
        assert_eq!(XtfOrdinal_4(&other, &IID_XBTP_EXTENSION_METADATA, &mut out), E_UNEXPECTED);
        assert!(out.is_null());
        assert_eq!(XtfOrdinal_4(&REMOTE_RUN_EXTENSION_ID, &other, &mut out), E_UNEXPECTED);
        assert!(out.is_null());
        assert_eq!(XtfOrdinal_4(&REMOTE_RUN_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut out), S_OK);
        assert!(!out.is_null());
        let mut n = 0u32;
        assert_eq!(com::metadata::get_handler_count(out, &mut n), S_OK);
        assert_eq!(n, 1);
        com::release(out);
    }
}

#[test]
fn run_handler_request_errors_report_console_lines() {
    let _g = guard();
    unsafe {
        let r = run_with("not json", &[]);
        let end = r.end.unwrap();
        assert!(end.contains("\"XtfOperationError\"") && end.contains("run.cpp : 925"), "{end}");
        let cases = [
            ("{}", 926u32),
            ("{\"dwFlags\":0}", 927),
            ("{\"dwFlags\":0,\"dwPeriod\":0}", 928),
            ("{\"dwFlags\":0,\"dwPeriod\":0,\"CommandLine\":\"x\"}", 929),
        ];
        for (req, line) in cases {
            let r = run_with(req, &[]);
            assert_eq!(r.end.as_deref(), Some(error_body(E_JSON_NOT_FOUND as u32, &[line]).as_str()), "{req}");
        }
        let r = run_with("{\"dwFlags\":0,\"dwPeriod\":0,\"CommandLine\":\"x\",\"WorkingDirectory\":\"\",\"Run_Error_Output\":\"yes\"}", &[]);
        let end = r.end.unwrap();
        assert!(end.contains("run.cpp : 932"), "{end}");
        assert!(r.sent.is_empty());
    }
}

#[test]
fn run_handler_without_console_jobs_fails_like_console_host() {
    let _g = guard();
    unsafe {
        let want = (STATUS_OBJECT_NAME_NOT_FOUND | FACILITY_NT_BIT) as i32;
        assert_eq!(want as u32, 0xD000_0034);
        let r = run_with(&request(1, 0, &child_cmd("child_marker_main"), "", false), &[]);
        assert_eq!(r.end.as_deref(), Some(error_body(want as u32, &[]).as_str()));
        let r = run_with(&request(5, 0, &child_cmd("child_marker_main"), "", false), &[]);
        assert_eq!(r.end.as_deref(), Some(error_body(want as u32, &[]).as_str()));
        register_job(NT_TITLE_JOB, GetCurrentProcess());
        let r = run_with(&request(1, 0, &child_cmd("child_marker_main"), "", false), &[]);
        unregister_job(NT_TITLE_JOB);
        let e = hr_from_wait(ERROR_FILE_NOT_FOUND) as u32;
        assert_eq!(r.end.as_deref(), Some(error_body(e, &[]).as_str()));
    }
}

#[test]
fn create_process_maps_images_and_directories_through_the_sandbox() {
    let _g = guard();
    let _j = Jobs::title();
    unsafe {
        let r = run_with(&request(1, 0, "C:\\tools\\absent.exe -x", "", false), &[]);
        assert_eq!(r.end.as_deref(), Some(error_body(hr_from_wait(ERROR_FILE_NOT_FOUND) as u32, &[]).as_str()));
        let dir = root().join("console").join("fs").join("C").join("tools");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("present.exe"), b"MZ").unwrap();
        let r = run_with(&request(1, 0, "C:\\tools\\present -x", "", false), &[]);
        assert_eq!(r.end.as_deref(), Some(error_body(hr_from_wait(ERROR_NOT_SUPPORTED) as u32, &[]).as_str()));
        let r = run_with(&request(1, 0, &child_cmd("child_marker_main"), "C:\\no\\such\\dir", false), &[]);
        assert_eq!(r.end.as_deref(), Some(error_body(hr_from_wait(ERROR_DIRECTORY) as u32, &[]).as_str()));
        assert_eq!(resolve_image("\"D:\\DevelopmentFiles\\xtfremoterun_child\" a b").unwrap(), format!("\"{}\" a b", std::env::current_exe().unwrap().display()));
        assert_eq!(console_sandbox::system::expand("%SystemRoot%\\System32"), "C:\\Windows\\System32");
    }
}

#[test]
fn run_without_wait_launches_and_replies_without_body() {
    let _g = guard();
    let _j = Jobs::title();
    unsafe {
        let dir = root().join("console").join("fs").join("C").join("markers");
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("xtfremoterun_marker.txt"));
        let r = run_with(&request(0, 0, &child_cmd("child_marker_main"), "C:\\markers", false), &[]);
        assert_eq!(r.end, None);
        assert!(r.sent.is_empty() && r.flags.is_empty());
        let pid = wait_for("marker file", 30, || std::fs::read_to_string(dir.join("xtfremoterun_marker.txt")).ok().filter(|s| !s.is_empty()));
        assert_ne!(pid, std::process::id().to_string());
    }
}

#[test]
fn run_with_wait_reports_exit_code_and_default_directory() {
    let _g = guard();
    let _j = Jobs::title();
    unsafe {
        let win = root().join("console").join("fs").join("C").join("Windows");
        let _ = std::fs::remove_file(win.join("xtfremoterun_marker.txt"));
        let r = run_with(&request(1, 0xffff_ffff, &child_cmd("child_marker_main"), "", false), &[]);
        assert_eq!(r.end.as_deref(), Some("{\"ExitCode\":5}"));
        assert!(r.sent.is_empty() && r.flags.is_empty());
        assert!(win.join("xtfremoterun_marker.txt").is_file());
    }
}

#[test]
fn run_redirects_stdout_and_separate_stderr() {
    let _g = guard();
    let _j = Jobs::title();
    unsafe {
        let r = run_with(&request(RUN_FLAG_REDIRECT_OUTPUT | 1, 20, &child_cmd("child_streams_main"), "", true), &[]);
        assert_eq!(r.end.as_deref(), Some("{\"ExitCode\":3}"));
        let out = joined(&r.sent, MSGID_RUN_OUTPUT, P_RUN_OUTPUT);
        let err = joined(&r.sent, MSGID_RUN_ERROR, P_RUN_ERROR_OUTPUT);
        assert!(out.contains("child out") && !out.contains("child err"), "{out:?}");
        assert!(err.contains("child err"), "{err:?}");
        assert!(r.flags.iter().all(|&f| f == 0) && !r.flags.is_empty());
        let r = run_with(&request(RUN_FLAG_REDIRECT_OUTPUT | 1, 20, &child_cmd("child_streams_main"), "", false), &[]);
        assert_eq!(r.end.as_deref(), Some("{\"ExitCode\":3}"));
        let out = joined(&r.sent, MSGID_RUN_OUTPUT, P_RUN_OUTPUT);
        assert!(out.contains("child out") && out.contains("child err"), "{out:?}");
        assert!(r.sent.iter().all(|s| s.contains("\"MsgId\":1")));
    }
}

#[test]
fn run_forwards_input_messages_to_child_stdin() {
    let _g = guard();
    let _j = Jobs::title();
    unsafe {
        let flags = RUN_FLAG_REDIRECT_INPUT | RUN_FLAG_REDIRECT_OUTPUT | 1;
        let r = run_with(&request(flags, 20, &child_cmd("child_echo_main"), "", false), &[Ok("{\"MsgId\":7}"), Ok("{\"MsgId\":0,\"Run_Input\":\"ping\\n\"}")]);
        assert_eq!(r.end.as_deref(), Some("{\"ExitCode\":0}"));
        let out = joined(&r.sent, MSGID_RUN_OUTPUT, P_RUN_OUTPUT);
        assert!(out.contains("got:ping"), "{out:?}");
    }
}

#[test]
fn run_stops_on_result_message_and_reports_negative_results() {
    let _g = guard();
    let _j = Jobs::title();
    unsafe {
        let r = run_with(&request(1, 20, &child_cmd("child_wait_main"), "", false), &[Ok("{\"MsgId\":2,\"Result\":0}")]);
        assert_eq!(r.end, None);
        assert!(r.elapsed < Duration::from_secs(20));
        let r = run_with(&request(1, 20, &child_cmd("child_wait_main"), "", false), &[Ok("{\"MsgId\":2,\"Result\":-2147467259}")]);
        assert_eq!(r.end.as_deref(), Some(error_body(E_FAIL as u32, &[1057]).as_str()));
        let r = run_with(&request(1, 20, &child_cmd("child_wait_main"), "", false), &[Ok("{\"MsgId\":2}")]);
        assert_eq!(r.end.as_deref(), Some(error_body(E_JSON_NOT_FOUND as u32, &[1056]).as_str()));
        let r = run_with(&request(1, 20, &child_cmd("child_wait_main"), "", false), &[Ok("{\"Other\":1}")]);
        assert_eq!(r.end.as_deref(), Some(error_body(E_JSON_NOT_FOUND as u32, &[1046]).as_str()));
        let r = run_with(&request(1, 20, &child_cmd("child_wait_main"), "", false), &[Ok("not json")]);
        assert!(r.end.as_deref().unwrap().contains("run.cpp : 1044"));
        let r = run_with(&request(1, 20, &child_cmd("child_wait_main"), "", false), &[Err(E_FAIL)]);
        assert_eq!(r.end.as_deref(), Some(error_body(E_FAIL as u32, &[1068]).as_str()));
    }
}

#[test]
fn console_pipe_helpers_round_trip_ansi_text() {
    let _g = guard();
    unsafe {
        let mut r: HANDLE = null_mut();
        let mut wr: HANDLE = null_mut();
        assert_ne!(CreatePipe(&mut r, &mut wr, core::ptr::null(), 0), 0);
        let (r, wr) = (OwnedHandle(r), OwnedHandle(wr));
        let mut b: BSTR = null_mut();
        assert_eq!(read_console_w(r.get(), &mut b), S_OK);
        assert!(b.is_null());
        let t = wide("hello\r\nworld");
        assert_eq!(write_console_w(wr.get(), t.as_ptr()), S_OK);
        let e = wide("");
        assert_eq!(write_console_w(wr.get(), e.as_ptr()), S_OK);
        assert_eq!(read_console_w(r.get(), &mut b), S_OK);
        assert_eq!(from_wide(b), "hello\r\nworld");
        free_bstr(b);
        let s = fixture::session();
        let mut any = true;
        assert_eq!(send_console_output(r.get(), fixture::as_obj(s), Some(&mut any)), S_OK);
        assert!(!any);
        assert!((*s).sent.lock().unwrap().is_empty());
        assert_eq!(write_console(wr.get(), b"abc\0def"), S_OK);
        assert_eq!(send_console_error(r.get(), fixture::as_obj(s), Some(&mut any)), S_OK);
        assert!(any);
        assert_eq!((*s).sent.lock().unwrap().clone(), vec!["{\"MsgId\":3,\"Run_Error_Output\":\"abc\"}".to_string()]);
        com::release(fixture::as_obj(s));
        let m = message_with_json("{\"MsgId\":0,\"Run_Input\":\"xyz\"}");
        assert_eq!(recv_console_input(wr.get(), m.get()), S_OK);
        assert_eq!(read_console_w(r.get(), &mut b), S_OK);
        assert_eq!(from_wide(b), "xyz");
        free_bstr(b);
        let m = message_with_json("{\"MsgId\":0}");
        assert_eq!(recv_console_input(wr.get(), m.get()), E_JSON_NOT_FOUND);
        drop(r);
        let t = wide("x");
        let hr = write_console_w(wr.get(), t.as_ptr());
        assert!(hr == hr_from_wait(232) || hr == hr_from_wait(109), "0x{:08X}", hr as u32);
    }
}

