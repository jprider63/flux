#![allow(unused)]
#![flux::cfg(scrape_quals = true)]

#[path = "../lib/rvec.rs"]
pub mod rvec;
use rvec::RVec;

#[flux::sig(fn (b:bool[true]))]
pub fn assert(b:bool) {
  if !b { panic!("assertion failed") }
}

#[flux::sig(fn(xs:RVec<T>[@xl], ys:RVec<T>[@yl]) -> RVec<T>[xl + yl])]
fn merge<T:Copy>(xs:RVec<T>, ys:RVec<T>) -> RVec<T> {
    let mut r = RVec::new();
    let mut i = 0;
    while i < xs.len() {
        r.push(xs[i]);
        i += 1;
    }
    i = 0;
    while i < ys.len() {
        r.push(ys[i]);
        i += 1;
    }
    r
}