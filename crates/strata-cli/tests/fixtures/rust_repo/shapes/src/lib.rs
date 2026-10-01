pub mod util;

pub struct Circle {
    r: f64,
}

pub enum Kind {
    Round,
}

pub trait Shape {
    fn area(&self) -> f64;
    fn describe(&self) -> String {
        format!("{}", self.area())
    }
}

impl Circle {
    pub fn new(r: f64) -> Self {
        Circle { r }
    }
    fn unit() -> f64 {
        1.0
    }
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        util::square(self.r) + Self::unit()
    }
}

impl Circle {
    fn twice(&self) -> f64 {
        self
            .area()
            * 2.0
    }
}

#[derive(Default)]
pub struct Config {
    n: u32,
}

pub fn config() -> Config {
    Config::default()
}

pub use util::make;

impl Circle {
    fn thrice(&self) -> f64 {
        self
            .inner
            .area()
    }
}

pub type Radius = f64;

#[derive(Clone)]
pub struct Marker;

impl Marker {
    fn dup(&self) -> Self {
        self.clone()
    }
}

nix::ioctl_none!(tun_none, 0x54, 1);

pub fn use_ioctl() {
    tun_none();
}

pub fn ident<T: Shape>(x: T) -> T {
    x
}

pub enum Shape2 {
    Sq { side: f64 },
    Pt,
}

pub fn make_sq() -> Shape2 {
    Shape2::Sq { side: 1.0 }
}

pub fn kind_ref() -> Kind {
    Kind::Round
}

pub fn kind_match(k: Kind) -> i32 {
    match k {
        Kind::Round => 1,
    }
}

pub struct Gauge {
    level: f64,
}

impl Gauge {
    pub fn level(&self) -> f64 {
        self.level
    }
}

pub fn read(g: &Gauge) -> f64 {
    g.level()
}

pub fn via_inline() {
    util::inline_tests::t();
}

nix::ioctl_readwrite_bad!(tun_rw, 0x54, Gauge);

pub fn via_inline_missing() {
    util::inline_tests::nope();
}
