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
