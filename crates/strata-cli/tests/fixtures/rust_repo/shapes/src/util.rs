use crate::Circle;

pub fn square(x: f64) -> f64 {
    x * x
}

pub fn make() -> Circle {
    let c = Circle::new(1.0);
    println!("made");
    c
}
