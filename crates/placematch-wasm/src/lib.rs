//! Plain C-ABI WASM wrapper (no wasm-bindgen). Strings in and out as UTF-8 in linear memory.
//! JS: am_alloc(n) -> ptr; write bytes; am_pick(q_ptr, q_len, cands_ptr, cands_len, cc_ptr, cc_len)
//! where candidates are joined with '\u{1}'. Result: >=0 index, -1 none, -2 ambiguous.
use placematch::{Matcher, Pick, Verdict};
use std::cell::RefCell;

thread_local! { static M: RefCell<Option<Matcher>> = const { RefCell::new(None) }; }

#[no_mangle]
pub extern "C" fn am_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

#[no_mangle]
pub unsafe extern "C" fn am_free(p: *mut u8, n: usize) {
    drop(Vec::from_raw_parts(p, 0, n));
}

#[no_mangle]
pub extern "C" fn am_init() {
    M.with(|m| {
        if m.borrow().is_none() {
            *m.borrow_mut() = Some(Matcher::new());
        }
    });
}

unsafe fn s<'a>(p: *const u8, n: usize) -> &'a str {
    std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, n))
}

#[no_mangle]
pub unsafe extern "C" fn am_pick(qp: *const u8, ql: usize, cp: *const u8, cl: usize, ccp: *const u8, ccl: usize) -> i32 {
    am_init();
    let q = s(qp, ql);
    let cands: Vec<&str> = s(cp, cl).split('\u{1}').collect();
    let cc = if ccl == 0 { None } else { Some(s(ccp, ccl)) };
    M.with(|m| match m.borrow().as_ref().unwrap().pick(q, &cands, cc) {
        Pick::Match(i) => i as i32,
        Pick::None => -1,
        Pick::Ambiguous(_) => -2,
    })
}

#[no_mangle]
pub unsafe extern "C" fn am_compare(qp: *const u8, ql: usize, cp: *const u8, cl: usize, ccp: *const u8, ccl: usize) -> i32 {
    am_init();
    let cc = if ccl == 0 { None } else { Some(s(ccp, ccl)) };
    M.with(|m| match m.borrow().as_ref().unwrap().compare(s(qp, ql), s(cp, cl), cc).0 {
        Verdict::Same => 1,
        Verdict::Different => 0,
        Verdict::Unknown => 2,
    })
}
