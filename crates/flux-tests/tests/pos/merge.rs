#![allow(unused)]
#![flux::cfg(scrape_quals = true)]

// #[path = "../lib/rvec.rs"]
// pub mod rvec;
// use rvec::RVec;
// 
// #[flux::sig(fn (b:bool[true]))]
// pub fn assert(b:bool) {
//   if !b { panic!("assertion failed") }
// }
// 
// #[flux::sig(fn(xs:RVec<T>[@xl], ys:RVec<T>[@yl]) -> RVec<T>[xl + yl])]
// fn concat<T:Copy>(xs:RVec<T>, ys:RVec<T>) -> RVec<T> {
//     let mut r = RVec::new();
//     let mut i = 0;
//     while i < xs.len() {
//         r.push(xs[i]);
//         i += 1;
//     }
//     i = 0;
//     while i < ys.len() {
//         r.push(ys[i]);
//         i += 1;
//     }
//     r
// }

fn foo<T: PartialOrd>(x: T, y: T) -> bool {
    x < y
}

// #[flux::sig(fn(xs:RVec<T>[@xl], ys:RVec<T>[@yl]) -> RVec<T>[xl + yl])]
// fn merge<T:Copy + PartialOrd>(xs:RVec<T>, ys:RVec<T>) -> RVec<T> {
//     let mut r = RVec::new();
//     let mut i = 0;
//     let mut j = 0;
//     while i + j < xs.len() + ys.len() {
//         if i == xs.len() {
//             r.push(ys[j]);
//             j += 1;
//         } else if j == ys.len() {
//             r.push(xs[i]);
//             i += 1;
//         } else {
//             if xs[i] < ys[j] {
//                 r.push(xs[i]);
//                 i += 1;
//             } else {
//                 r.push(ys[j]);
//                 j += 1;
//             }
//         }
//     }
//     r
// }
