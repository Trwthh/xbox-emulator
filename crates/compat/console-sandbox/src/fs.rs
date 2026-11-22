use core::ffi::c_void;
use std::path::PathBuf;

use crate::wstr::{from_ptr, put_wide, wide};

pub const WINDOWS_DIR: &str = "C:\\Windows";
pub const CURRENT_DIR: &str = "C:\\Windows\\System32";
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
const INVALID_HANDLE_VALUE: isize = -1;
const FILE_RENAME_INFO: u32 = 3;
const FILE_RENAME_INFO_EX: u32 = 22;

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disposition: u32, flags: u32, template: isize) -> isize;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn SetFileInformationByHandle(h: isize, class: u32, info: *const u8, size: u32) -> i32;
    fn SetLastError(e: u32);
}

pub fn fs_root() -> PathBuf {
    crate::root().join("fs")
}

pub fn drive_dir(letter: char) -> PathBuf {
    fs_root().join(letter.to_ascii_uppercase().to_string())
}

pub fn ensure_system_drive() {
    let _ = std::fs::create_dir_all(drive_dir('C').join("Windows").join("System32"));
}

pub fn resolve(console: &str) -> Option<(String, PathBuf)> {
    let mut s = console.replace('/', "\\");
    for p in ["\\\\?\\", "\\??\\"] {
        if let Some(r) = s.strip_prefix(p) {
            s = r.to_string();
        }
    }
    if s.starts_with("\\\\") || s.is_empty() {
        return None;
    }
    let b = s.as_bytes();
    let (drive, rest) = if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        let rest = &s[2..];
        let rest = if rest.starts_with('\\') { rest.to_string() } else { format!("\\{rest}") };
        ((b[0] as char).to_ascii_uppercase(), rest)
    } else if s.starts_with('\\') {
        ('C', s.clone())
    } else {
        ('C', format!("{}\\{}", &CURRENT_DIR[2..], s))
    };
    let mut comps: Vec<&str> = Vec::new();
    for c in rest.split('\\') {
        match c {
            "" | "." => {}
            ".." => {
                comps.pop();
            }
            c => comps.push(c),
        }
    }
    let mut host = drive_dir(drive);
    for c in &comps {
        host.push(c);
    }
    Some((format!("{drive}:\\{}", comps.join("\\")), host))
}

fn host_wide(console: *const u16) -> Option<(String, Vec<u16>)> {
    let (c, h) = resolve(&unsafe { from_ptr(console) })?;
    Some((c, wide(&h.to_string_lossy())))
}

pub unsafe extern "system" fn create_file_w(name: *const u16, access: u32, share: u32, sa: *const c_void, disposition: u32, flags: u32, template: isize) -> isize {
    if !name.is_null() && crate::logan::is_device_path(&from_ptr(name)) {
        return crate::logan::open();
    }
    if !name.is_null() && crate::vab::is_device_path(&from_ptr(name)) {
        return crate::vab::open();
    }
    match host_wide(name) {
        Some((_, h)) => CreateFileW(h.as_ptr(), access, share, sa, disposition, flags, template),
        None => {
            SetLastError(ERROR_PATH_NOT_FOUND);
            INVALID_HANDLE_VALUE
        }
    }
}

pub unsafe extern "system" fn get_file_attributes_w(name: *const u16) -> u32 {
    match host_wide(name) {
        Some((_, h)) => GetFileAttributesW(h.as_ptr()),
        None => {
            SetLastError(ERROR_PATH_NOT_FOUND);
            u32::MAX
        }
    }
}

pub unsafe extern "system" fn delete_file_w(name: *const u16) -> i32 {
    match host_wide(name) {
        Some((_, h)) => DeleteFileW(h.as_ptr()),
        None => {
            SetLastError(ERROR_PATH_NOT_FOUND);
            0
        }
    }
}

pub unsafe extern "system" fn set_file_information_by_handle(h: isize, class: u32, info: *const u8, size: u32) -> i32 {
    if (class != FILE_RENAME_INFO && class != FILE_RENAME_INFO_EX) || info.is_null() || size < 20 {
        return SetFileInformationByHandle(h, class, info, size);
    }
    let len = (info.add(16) as *const u32).read_unaligned() as usize / 2;
    let name: Vec<u16> = std::slice::from_raw_parts(info.add(20) as *const u16, len).to_vec();
    let Some((_, host)) = resolve(&String::from_utf16_lossy(&name)) else {
        SetLastError(ERROR_PATH_NOT_FOUND);
        return 0;
    };
    let hw: Vec<u16> = host.to_string_lossy().encode_utf16().collect();
    let mut buf = vec![0u8; 20 + hw.len() * 2 + 2];
    buf[..16].copy_from_slice(std::slice::from_raw_parts(info, 16));
    buf[16..20].copy_from_slice(&((hw.len() * 2) as u32).to_le_bytes());
    for (i, c) in hw.iter().enumerate() {
        buf[20 + i * 2..22 + i * 2].copy_from_slice(&c.to_le_bytes());
    }
    let mut aligned = vec![0u64; buf.len().div_ceil(8)];
    core::ptr::copy_nonoverlapping(buf.as_ptr(), aligned.as_mut_ptr() as *mut u8, buf.len());
    SetFileInformationByHandle(h, class, aligned.as_ptr() as *const u8, buf.len() as u32)
}

pub unsafe extern "system" fn get_drive_type_w(root: *const u16) -> u32 {
    let r = if root.is_null() { "C:\\".to_string() } else { from_ptr(root) };
    let b = r.as_bytes();
    if b.len() < 2 || b[1] != b':' || !(b.len() == 2 || (b[2] == b'\\' && b.len() == 3)) {
        return 1;
    }
    if drive_dir(b[0] as char).is_dir() {
        3
    } else {
        1
    }
}

pub unsafe extern "system" fn get_system_windows_directory_w(buf: *mut u16, size: u32) -> u32 {
    let w: Vec<u16> = WINDOWS_DIR.encode_utf16().collect();
    if buf.is_null() || (size as usize) < w.len() + 1 {
        return w.len() as u32 + 1;
    }
    put_wide(buf, &w);
    w.len() as u32
}

pub unsafe extern "system" fn search_path_w(path: *const u16, file: *const u16, ext: *const u16, len: u32, buf: *mut u16, part: *mut *mut u16) -> u32 {
    let f = from_ptr(file);
    if f.is_empty() {
        SetLastError(87);
        return 0;
    }
    let e = from_ptr(ext);
    let names: Vec<String> = if !e.is_empty() && !f.rsplit('\\').next().unwrap_or("").contains('.') { vec![format!("{f}{e}")] } else { vec![f.clone()] };
    let has_dir = f.contains('\\') || f.contains(':');
    let dirs: Vec<String> = if has_dir {
        vec![String::new()]
    } else if !path.is_null() {
        from_ptr(path).split(';').filter(|d| !d.is_empty()).map(|d| d.to_string()).collect()
    } else {
        vec![CURRENT_DIR.to_string(), WINDOWS_DIR.to_string()]
    };
    for d in &dirs {
        for n in &names {
            let full = if d.is_empty() { n.clone() } else { format!("{}\\{n}", d.trim_end_matches('\\')) };
            if let Some((console, host)) = resolve(&full) {
                if host.is_file() || host.is_dir() {
                    let w: Vec<u16> = console.encode_utf16().collect();
                    if buf.is_null() || (len as usize) < w.len() + 1 {
                        return w.len() as u32 + 1;
                    }
                    put_wide(buf, &w);
                    if !part.is_null() {
                        let slash = w.iter().rposition(|&c| c == b'\\' as u16).map(|i| i + 1).unwrap_or(0);
                        *part = buf.add(slash);
                    }
                    return w.len() as u32;
                }
            }
        }
    }
    SetLastError(ERROR_FILE_NOT_FOUND);
    0
}
