/*
use libsave3ds::save_data::*;
use libsave3ds::*;
use std::boxed::Box;
use std::ffi::CStr;
use std::mem::drop;
use std::os::raw::c_char;
use std::ptr::null_mut;
use std::rc::Rc;
use std::slice;

fn to_raw<T, U>(x: Result<T, U>) -> *mut T {
    if let Ok(r) = x {
        Box::into_raw(Box::new(r))
    } else {
        null_mut()
    }
}

unsafe fn from_raw<T>(x: *mut T) -> Box<T> {
    Box::from_raw(x)
}

unsafe fn release_raw<T>(x: *mut T) {
    drop(Box::from_raw(x));
}

unsafe fn from_c_char(s: *const c_char) -> Option<&'static str> {
    if s.is_null() {
        None
    } else {
        Some(CStr::from_ptr(s).to_str().unwrap())
    }
}

fn smash_error<U>(x: Result<(), U>) -> i32 {
    if x.is_ok() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_resource_create(
    boot9_path: *const c_char,
    movable_path: *const c_char,
    sd_path: *const c_char,
    nand_path: *const c_char,
) -> *mut Resource {
    let boot9_path = from_c_char(boot9_path).map(str::to_owned);
    let movable_path = from_c_char(movable_path).map(str::to_owned);
    let sd_path = from_c_char(sd_path).map(str::to_owned);
    let nand_path = from_c_char(nand_path).map(str::to_owned);

    to_raw(Resource::new(boot9_path, movable_path, sd_path, nand_path))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_resource_release(resource: *mut Resource) {
    release_raw(resource);
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_open_sd_save(
    resource: *mut Resource,
    id: u64,
) -> *mut Rc<SaveData> {
    to_raw((*resource).open_sd_save(id))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_open_nand_save(
    resource: *mut Resource,
    id: u32,
) -> *mut Rc<SaveData> {
    to_raw((*resource).open_nand_save(id))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_open_bare_save(
    resource: *mut Resource,
    path: *const c_char,
) -> *mut Rc<SaveData> {
    to_raw((*resource).open_bare_save(from_c_char(path).unwrap()))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_release(save: *mut Rc<SaveData>) {
    release_raw(save);
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_commit(save: *mut Rc<SaveData>) -> i32 {
    smash_error((*save).commit())
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_open_ino(
    save: *mut Rc<SaveData>,
    ino: u32,
) -> *mut File {
    to_raw(File::open_ino((*save).clone(), ino))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_release(file: *mut File) {
    release_raw(file);
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_rename(
    file: *mut File,
    parent: *mut Dir,
    name: *const [u8; 16],
) -> i32 {
    smash_error((*file).rename(&*parent, *name))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_get_parent_ino(file: *mut File) -> u32 {
    (*file).get_parent_ino()
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_get_ino(file: *mut File) -> u32 {
    (*file).get_ino()
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_delete(file: *mut File) -> i32 {
    smash_error(from_raw(file).delete())
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_resize(file: *mut File, len: u64) -> i32 {
    smash_error((*file).resize(len as usize))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_len(file: *mut File) -> u64 {
    (*file).len() as u64
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_read(
    file: *mut File,
    pos: u64,
    len: u64,
    buf: *mut u8,
) -> i32 {
    smash_error((*file).read(pos as usize, slice::from_raw_parts_mut(buf, len as usize)))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_file_write(
    file: *mut File,
    pos: u64,
    len: u64,
    buf: *const u8,
) -> i32 {
    smash_error((*file).write(pos as usize, slice::from_raw_parts(buf, len as usize)))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_open_root(save: *mut Rc<SaveData>) -> *mut Dir {
    to_raw(Dir::open_root((*save).clone()))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_open_ino(save: *mut Rc<SaveData>, ino: u32) -> *mut Dir {
    to_raw(Dir::open_ino((*save).clone(), ino))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_release(dir: *mut Dir) {
    release_raw(dir);
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_rename(
    dir: *mut Dir,
    parent: *mut Dir,
    name: *const [u8; 16],
) -> i32 {
    smash_error((*dir).rename(&*parent, *name))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_get_parent_ino(dir: *mut Dir) -> u32 {
    (*dir).get_parent_ino()
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_get_ino(dir: *mut Dir) -> u32 {
    (*dir).get_ino()
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_delete(dir: *mut Dir) -> i32 {
    smash_error(from_raw(dir).delete())
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_open_sub_dir(
    dir: *mut Dir,
    name: *const [u8; 16],
) -> *mut Dir {
    to_raw((*dir).open_sub_dir(*name))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_open_sub_file(
    dir: *mut Dir,
    name: *const [u8; 16],
) -> *mut File {
    to_raw((*dir).open_sub_file(*name))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_new_sub_dir(
    dir: *mut Dir,
    name: *const [u8; 16],
) -> *mut Dir {
    to_raw((*dir).new_sub_dir(*name))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_new_sub_file(
    dir: *mut Dir,
    name: *const [u8; 16],
    len: u64,
) -> *mut File {
    to_raw((*dir).new_sub_file(*name, len as usize))
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_list_sub_dir(dir: *mut Dir) -> *mut Vec<([u8; 16], u32)> {
    to_raw((*dir).list_sub_dir())
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_save_dir_list_sub_file(
    dir: *mut Dir,
) -> *mut Vec<([u8; 16], u32)> {
    to_raw((*dir).list_sub_file())
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_entry_list_release(list: *mut Vec<([u8; 16], u32)>) {
    release_raw(list)
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_entry_list_len(list: *mut Vec<([u8; 16], u32)>) -> u32 {
    (*list).len() as u32
}

#[no_mangle]
pub unsafe extern "C" fn save3ds_entry_list_get(
    list: *mut Vec<([u8; 16], u32)>,
    index: u32,
    name: *mut [u8; 16],
    ino: *mut u32,
) {
    let entry = &(*list)[index as usize];
    *name = entry.0;
    *ino = entry.1;
}
*/
