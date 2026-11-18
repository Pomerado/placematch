//! Plain C-ABI WASM wrapper (no wasm-bindgen). Strings in and out as UTF-8 in linear memory.
//! JS: am_alloc(n) -> ptr; write bytes; am_pick(q_ptr, q_len, cands_ptr, cands_len, cc_ptr, cc_len)
//! where candidates are joined with '\u{1}'. Result: >=0 index, -1 none, -2 ambiguous.
//! am_pick_explained(q, cands, cc, flags) takes one byte per row in `flags` ('1': the site
//! categorized the row) and returns a pointer to the answer's JSON; am_out_len() is its length.
//! am_pick_explained_with(q, cands, cc, flags, rules) is am_pick_explained under optional rules:
//! `rules` names them, comma separated ("district_may_differ"); "" is none. A name it does not know
//! is refused: it returns a null pointer and am_out_len() is 0.
//! The picker checks (am_is_control, am_shows_typed_text, am_in_area, am_same_but_postcode) take
//! the query, the row and a third text and return 1 or 0.
use placematch::{Matcher, Pick, PickOptions, Row, Rules, Verdict};
use std::cell::RefCell;

thread_local! {
    static M: RefCell<Option<Matcher>> = const { RefCell::new(None) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

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

#[no_mangle]
pub unsafe extern "C" fn am_pick_explained(qp: *const u8, ql: usize, cp: *const u8, cl: usize, ccp: *const u8, ccl: usize, fp: *const u8, fl: usize) -> *const u8 {
    pick_explained(qp, ql, cp, cl, ccp, ccl, fp, fl, &Rules::default())
}

#[no_mangle]
pub unsafe extern "C" fn am_pick_explained_with(qp: *const u8, ql: usize, cp: *const u8, cl: usize, ccp: *const u8, ccl: usize, fp: *const u8, fl: usize, rp: *const u8, rl: usize) -> *const u8 {
    let mut rules = Rules::default();
    for name in s(rp, rl).split(',').filter(|name| !name.is_empty()) {
        match name {
            "district_may_differ" => rules.district_may_differ = true,
            _ => {
                OUT.with(|o| o.borrow_mut().clear());
                return std::ptr::null();
            }
        }
    }
    pick_explained(qp, ql, cp, cl, ccp, ccl, fp, fl, &rules)
}

#[allow(clippy::too_many_arguments)]
unsafe fn pick_explained(qp: *const u8, ql: usize, cp: *const u8, cl: usize, ccp: *const u8, ccl: usize, fp: *const u8, fl: usize, rules: &Rules) -> *const u8 {
    am_init();
    let flags = std::slice::from_raw_parts(fp, fl);
    let rows: Vec<Row> = s(cp, cl).split('\u{1}').enumerate().map(|(i, text)| Row { text, categorized: flags.get(i) == Some(&b'1') }).collect();
    let country = if ccl == 0 { None } else { Some(s(ccp, ccl)) };
    let json = M.with(|m| m.borrow().as_ref().unwrap().pick_explained_with(s(qp, ql), &rows, &PickOptions { country }, rules).to_json());
    OUT.with(|o| {
        *o.borrow_mut() = json.into_bytes();
        o.borrow().as_ptr()
    })
}

/// Length of the last answer `am_pick_explained` wrote.
#[no_mangle]
pub extern "C" fn am_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

/// The four picker checks: `am_is_control`, `am_shows_typed_text` (the third text is the least
/// number of words, in decimal), `am_in_area` and `am_same_but_postcode` (the third text is the
/// country). Each returns 1 for yes and 0 for no.
fn check(qp: *const u8, ql: usize, rp: *const u8, rl: usize, xp: *const u8, xl: usize, f: impl Fn(&Matcher, &str, &str, Option<&str>) -> bool) -> i32 {
    am_init();
    let (q, r) = unsafe { (s(qp, ql), s(rp, rl)) };
    let x = if xl == 0 { None } else { Some(unsafe { s(xp, xl) }) };
    M.with(|m| f(m.borrow().as_ref().unwrap(), q, r, x) as i32)
}

#[no_mangle]
pub extern "C" fn am_is_control(qp: *const u8, ql: usize, rp: *const u8, rl: usize, xp: *const u8, xl: usize) -> i32 {
    check(qp, ql, rp, rl, xp, xl, |m, q, r, _| m.is_control(q, r))
}

#[no_mangle]
pub extern "C" fn am_shows_typed_text(qp: *const u8, ql: usize, rp: *const u8, rl: usize, xp: *const u8, xl: usize) -> i32 {
    check(qp, ql, rp, rl, xp, xl, |m, q, r, n| m.shows_typed_text(q, r, n.and_then(|n| n.parse().ok()).unwrap_or(1)))
}

#[no_mangle]
pub extern "C" fn am_in_area(qp: *const u8, ql: usize, rp: *const u8, rl: usize, xp: *const u8, xl: usize) -> i32 {
    check(qp, ql, rp, rl, xp, xl, |m, q, r, cc| m.in_area(q, r, cc))
}

#[no_mangle]
pub extern "C" fn am_same_but_postcode(qp: *const u8, ql: usize, rp: *const u8, rl: usize, xp: *const u8, xl: usize) -> i32 {
    check(qp, ql, rp, rl, xp, xl, |m, q, r, cc| m.same_but_postcode(q, r, cc))
}
