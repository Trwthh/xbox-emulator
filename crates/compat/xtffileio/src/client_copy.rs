use crate::com::*;
use core::ffi::c_void;

pub const E_NOTIMPL: HRESULT = 0x8000_4001u32 as i32;

type P = *mut c_void;
type W = *const u16;

pub const I3_CALCULATE_INCREMENTAL_LIST: usize = 0;

pub unsafe extern "system" fn c_copy_files(_this: P, _pattern: W, _include: u32, _exclude: u32, _recursion: u32, _dst: W, _flags: u32, _callback: P) -> HRESULT {
    E_NOTIMPL
}

pub unsafe extern "system" fn i2_copy_files(_this: P, _pattern: W, _include: u32, _exclude: u32, _recursion: u32, _dst: W, _flags: u32, _callback: P) -> HRESULT {
    E_NOTIMPL
}

pub unsafe extern "system" fn i3_copy_files_direct_list(_this: P, _list: P, _reserved: u32, _pattern: W, _dst: W, _flags: u32, _perf: P, _callback: P) -> HRESULT {
    E_NOTIMPL
}

pub unsafe extern "system" fn i3_copy_files_perf(_this: P, _pattern: W, _include: u32, _exclude: u32, _recursion: u32, _dst: W, _flags: u32, _perf: P, _callback: P) -> HRESULT {
    E_NOTIMPL
}
