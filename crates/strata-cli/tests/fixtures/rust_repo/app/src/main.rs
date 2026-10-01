use shapes::util::square;
use shapes::{Circle, Shape};

fn main() {
    let c = Circle::new(2.0);
    c.area();
    square(3.0);
    std::process::exit(0);
}

use shapes::util;

fn shadow() {
    let util = Circle::new(1.0);
    util.area();
}

fn ext() {
    let mut v = Vec::new();
    let _o = Some(1);
    v.push(1);
    fn inner() {}
    inner();
    let f = || 1;
    f();
}

use shapes::make;

fn reexport() {
    make();
}

fn closure_in_macro() {
    let show = |x: i32| x;
    println!("{}", show(1));
}

fn qualified() {
    shapes::Config::default();
}

mod helper;

fn uses_helper() {
    helper::go();
}

fn guard(o: Option<i32>) -> bool {
    match o {
        Some(x) if helper_fn(x) => true,
        _ => false,
    }
}

fn helper_fn(_x: i32) -> bool {
    true
}
