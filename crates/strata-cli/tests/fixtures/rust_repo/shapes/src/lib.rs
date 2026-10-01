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
