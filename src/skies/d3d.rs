//! Thin wrappers over Windows' d3dcompiler_47.dll.

use anyhow::{bail, Context, Result};
use libloading::{Library, Symbol};
use std::ffi::{c_void, CString};
use std::ptr;
use std::sync::OnceLock;

type D3DCompileFn = unsafe extern "system" fn(
    *const u8,
    usize,
    *const i8,
    *const c_void,
    *const c_void,
    *const i8,
    *const i8,
    u32,
    u32,
    *mut *mut c_void,
    *mut *mut c_void,
) -> i32;

struct Api {
    _lib: Library,
    compile: D3DCompileFn,
}

fn api() -> Result<&'static Api> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    match API.get_or_init(|| load().map_err(|e| format!("{e:#}"))) {
        Ok(api) => Ok(api),
        Err(e) => bail!("{e}"),
    }
}

fn load() -> Result<Api> {
    unsafe {
        let lib = Library::new("d3dcompiler_47.dll").context("loading d3dcompiler_47.dll")?;
        let compile: Symbol<D3DCompileFn> = lib.get(b"D3DCompile\0").context("D3DCompile")?;
        Ok(Api { compile: *compile, _lib: lib })
    }
}

unsafe fn blob_bytes(blob: *mut c_void) -> Vec<u8> {
    if blob.is_null() {
        return Vec::new();
    }
    // ID3DBlob vtable: QueryInterface, AddRef, Release, GetBufferPointer, GetBufferSize
    let vtable = *(blob as *mut *mut usize);
    let get_pointer: unsafe extern "system" fn(*mut c_void) -> *const u8 = std::mem::transmute(*vtable.add(3));
    let get_size: unsafe extern "system" fn(*mut c_void) -> usize = std::mem::transmute(*vtable.add(4));
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(*vtable.add(2));
    let ptr = get_pointer(blob);
    let size = get_size(blob);
    let data = if ptr.is_null() || size == 0 { Vec::new() } else { std::slice::from_raw_parts(ptr, size).to_vec() };
    release(blob);
    data
}

/// Compiles HLSL to a DXBC blob for `vs_5_0` or `ps_5_0`.
pub fn compile(source: &str, entry: &str, target: &str) -> Result<Vec<u8>> {
    let api = api()?;
    let source_name = CString::new("sky").unwrap();
    let entry = CString::new(entry).unwrap();
    let target = CString::new(target).unwrap();
    let mut code: *mut c_void = ptr::null_mut();
    let mut errors: *mut c_void = ptr::null_mut();
    let result = unsafe {
        (api.compile)(
            source.as_ptr(),
            source.len(),
            source_name.as_ptr(),
            ptr::null(),
            ptr::null(),
            entry.as_ptr(),
            target.as_ptr(),
            0,
            0,
            &mut code,
            &mut errors,
        )
    };
    let message = unsafe { String::from_utf8_lossy(&blob_bytes(errors)).trim_matches('\0').to_string() };
    if result != 0 {
        bail!("D3DCompile failed: {message}");
    }
    Ok(unsafe { blob_bytes(code) })
}
