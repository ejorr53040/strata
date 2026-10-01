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

fn pick() -> i32 {
    1
}

fn shadow_call() {
    let pick = pick();
    let _ = pick;
}

fn json_like(config: Cfg) {
    let _v = serde_json::json!({
        "kernel_image_path": helper_fn(&config.kernel_image_path),
        "boot_args": config.boot_args,
    });
}

fn turbofish() {
    let _v = Vec::<String>::new();
}

pub struct Tup(pub i32);

fn generic_fn<T>() {}

fn types_in_odd_places(config: Cfg) {
    let _a = Tup(1);
    generic_fn::<Tup>();
    assert_eq!(Tup(2).0, 2);
    let _j = serde_json::json!({
        "a": Tup(3),
        "b": config.field,
    });
}

const LIMIT: i32 = 3;
static NAME: &str = "n";

fn double(x: i32) -> i32 {
    x * 2
}

struct Args {
    #[arg(value_parser = double)]
    n: i32,
}

fn consts_and_values() {
    let _x = LIMIT + 1;
    let _n = NAME.len();
    let _v: Vec<i32> = vec![1].into_iter().map(double).collect();
}

fn field_on_unknown(c: shapes::Circle) {
    let _ = c.r;
}

use shapes::Kind::*;

fn bare_variant(k: shapes::Kind) -> i32 {
    match k {
        Round => 1,
    }
}

fn local_items() -> i32 {
    static COUNTER: i32 = 1;
    COUNTER
}

struct Args2 {
    #[arg(default_value_t = i64::from(LIMIT))]
    n: i64,
}

fn tuple_pat() {
    let _f = |Tup(x)| x;
}

struct Wrap {
    inner: shapes::Gauge,
}

fn inner() {}

fn shorthand_local(w: Wrap) {
    let Wrap { inner } = w;
    let _l = inner.level;
}

struct Cfg2 {
    n_vcpus: u32,
}

fn json_field(config: Cfg2) {
    let _j = serde_json::json!({
        "kernel_image_path": config.n_vcpus,
        "b": 1,
    });
}

struct Guard2;

impl Drop for Guard2 {
    fn drop(&mut self) {}
}

fn takes_fn<F: Fn(i32) -> i32>(f: F) -> i32 {
    f(1)
}

const _: () = ();

fn infer() {
    let _v: Vec<_> = Vec::new();
}

struct Cfg {
    kernel_image_path: String,
    boot_args: String,
    field: i32,
    n: i32,
}

fn chain() -> Vec<i32> {
    vec![1].into_iter().map(double).collect::<Vec<_>>()
}

struct Holder {
    g: shapes::Gauge,
}

fn typed(h: Holder, g2: shapes::Gauge) {
    let _a = h.g.level();
    let c = shapes::Circle::new(1.0);
    let _b = c.area();
    let _d = g2.level();
}
